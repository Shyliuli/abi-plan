// AGENT SAFETY: 自动生成测试框架无需人类理解

use std::{collections::BTreeMap, env, fmt::Write as _, path::Path, process::Command};

pub struct DebugReport {
    pub mapping: String,
}

struct Die {
    offset: u64,
    tag: String,
    attrs: BTreeMap<String, String>,
}

struct Variable {
    name: String,
    ty: String,
    size: Option<u64>,
    frame_offset: i64,
    members: Vec<Member>,
}

struct Member {
    path: String,
    offset: u64,
    size: u64,
    ty: String,
}

struct FunctionInfo {
    variables: Vec<Variable>,
    return_type: String,
    return_size: Option<u64>,
}

#[derive(Clone)]
enum Operand {
    Register(String),
    Memory(i64),
    Other,
}

struct Move {
    source: Operand,
    destination: Operand,
}

pub fn render(
    clang: &str,
    source: &Path,
    function: &str,
    result_type: &str,
    work_dir: &Path,
) -> Result<DebugReport, String> {
    let object = work_dir.join(format!("{function}.o"));
    compile_object(clang, source, &object)?;

    let assembly = compile_assembly(clang, source)?;
    let function_assembly = extract_function(&assembly, function)
        .ok_or_else(|| format!("assembly for `{function}` was not found"))?;
    let dwarfdump = env::var("LLVM_DWARFDUMP").unwrap_or_else(|_| "llvm-dwarfdump".to_owned());
    let output = Command::new(&dwarfdump)
        .arg("--debug-info")
        .arg(&object)
        .output()
        .map_err(|error| format!("cannot run {dwarfdump}: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }

    let dies = parse_dies(&String::from_utf8_lossy(&output.stdout));
    let function_info = parse_function(&dies, function);
    Ok(DebugReport {
        mapping: render_mapping(&function_assembly, function, &function_info, result_type),
    })
}

fn compile_object(clang: &str, source: &Path, object: &Path) -> Result<(), String> {
    let output = Command::new(clang)
        .args([
            "--target=x86_64-unknown-linux-gnu",
            "-O0",
            "-g",
            "-ffreestanding",
            "-fno-stack-protector",
            "-fno-pic",
            "-c",
        ])
        .arg(source)
        .arg("-o")
        .arg(object)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn compile_assembly(clang: &str, source: &Path) -> Result<String, String> {
    let output = Command::new(clang)
        .args([
            "--target=x86_64-unknown-linux-gnu",
            "-O0",
            "-g",
            "-S",
            "-fverbose-asm",
            "-fno-asynchronous-unwind-tables",
            "-ffreestanding",
            "-fno-stack-protector",
            "-fno-pic",
        ])
        .arg(source)
        .args(["-o", "-"])
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn extract_function(assembly: &str, name: &str) -> Option<String> {
    let label = format!("{name}:");
    let mut lines = assembly.lines();
    let first = lines.find(|line| line.trim_start().starts_with(&label))?;
    let mut function = format!("{first}\n");
    for line in lines {
        if line.starts_with(".Lfunc_end") {
            break;
        }
        function.push_str(line);
        function.push('\n');
    }
    Some(function)
}

fn parse_dies(text: &str) -> Vec<Die> {
    let mut dies = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if let Some((offset, tag)) = parse_die_start(line) {
            dies.push(Die {
                offset,
                tag,
                attrs: BTreeMap::new(),
            });
            continue;
        }
        let Some(attribute) = line.strip_prefix("DW_AT_") else {
            continue;
        };
        let Some(split) = attribute.find(char::is_whitespace) else {
            continue;
        };
        let Some(die) = dies.last_mut() else {
            continue;
        };
        die.attrs.insert(
            format!("DW_AT_{}", &attribute[..split]),
            attribute[split..].trim().to_owned(),
        );
    }
    dies
}

fn parse_die_start(line: &str) -> Option<(u64, String)> {
    let (offset, rest) = line.split_once(':')?;
    let offset = u64::from_str_radix(offset.trim_start_matches("0x"), 16).ok()?;
    let rest = rest.trim_start();
    let tag = rest.strip_prefix("DW_TAG_")?;
    Some((offset, tag.split_whitespace().next()?.to_owned()))
}

fn parse_function(dies: &[Die], function: &str) -> FunctionInfo {
    let Some(start) = dies.iter().position(|die| {
        die.tag == "subprogram" && quoted(die, "DW_AT_name").as_deref() == Some(function)
    }) else {
        return FunctionInfo {
            variables: Vec::new(),
            return_type: "?".to_owned(),
            return_size: None,
        };
    };

    let variables = dies[start + 1..]
        .iter()
        .take_while(|die| die.tag != "subprogram" && die.tag != "compile_unit")
        .filter(|die| die.tag == "formal_parameter" || die.tag == "variable")
        .filter_map(|die| {
            let location = die.attrs.get("DW_AT_location")?;
            let frame_offset = frame_offset(location)?;
            let type_ref = hex_attribute(die, "DW_AT_type");
            let ty = die
                .attrs
                .get("DW_AT_type")
                .and_then(|value| quoted_value(value))
                .unwrap_or_else(|| "?".to_owned());
            let size = type_ref.and_then(|offset| type_size(dies, offset));
            let mut members =
                type_ref.map_or_else(Vec::new, |offset| collect_members(dies, offset, 0, "", 0));
            if ty == "complex" {
                if let Some(size) = size {
                    let half = size / 2;
                    let ty = if half == 4 { "float" } else { "double" }.to_owned();
                    members = vec![
                        Member {
                            path: "re".to_owned(),
                            offset: 0,
                            size: half,
                            ty: ty.clone(),
                        },
                        Member {
                            path: "im".to_owned(),
                            offset: half,
                            size: half,
                            ty,
                        },
                    ];
                }
            }
            Some(Variable {
                name: quoted(die, "DW_AT_name").unwrap_or_else(|| "<unnamed>".to_owned()),
                ty,
                size,
                frame_offset,
                members,
            })
        })
        .collect();

    let return_type_ref = hex_attribute(&dies[start], "DW_AT_type");
    FunctionInfo {
        variables,
        return_type: dies[start]
            .attrs
            .get("DW_AT_type")
            .and_then(|value| quoted_value(value))
            .unwrap_or_else(|| "void".to_owned()),
        return_size: return_type_ref.and_then(|offset| type_size(dies, offset)),
    }
}

fn frame_offset(location: &str) -> Option<i64> {
    let rest = location.split("DW_OP_fbreg").nth(1)?.trim_start();
    let value = rest
        .split(|character: char| character == ')' || character.is_whitespace())
        .next()?;
    value.parse().ok()
}

fn type_size(dies: &[Die], offset: u64) -> Option<u64> {
    let mut current = offset;
    for _ in 0..8 {
        let die = die_by_offset(dies, current)?;
        if die.tag == "pointer_type" {
            return Some(8);
        }
        if die.tag == "array_type" {
            let element_size =
                hex_attribute(die, "DW_AT_type").and_then(|element| type_size(dies, element))?;
            let count = array_count(dies, die.offset)?;
            return Some(element_size * count);
        }
        if let Some(size) = hex_attribute(die, "DW_AT_byte_size") {
            return Some(size);
        }
        current = hex_attribute(die, "DW_AT_type")?;
    }
    None
}

fn collect_members(
    dies: &[Die],
    type_offset: u64,
    base_offset: u64,
    prefix: &str,
    depth: usize,
) -> Vec<Member> {
    if depth >= 8 {
        return Vec::new();
    }
    let Some(type_die) = resolved_type(dies, type_offset) else {
        return Vec::new();
    };
    if let Some((count, element_size, element_type)) = array_info(dies, type_die.offset) {
        return (0..count)
            .map(|index| Member {
                path: format!("{prefix}[{index}]"),
                offset: base_offset + index * element_size,
                size: element_size,
                ty: element_type.clone(),
            })
            .collect();
    }
    if type_die.tag != "structure_type" && type_die.tag != "union_type" {
        return Vec::new();
    }
    let Some(index) = dies.iter().position(|die| die.offset == type_die.offset) else {
        return Vec::new();
    };
    let mut members = Vec::new();
    for member in dies[index + 1..]
        .iter()
        .take_while(|die| die.tag == "member")
    {
        let Some(name) = quoted(member, "DW_AT_name") else {
            continue;
        };
        let offset = hex_attribute(member, "DW_AT_data_member_location").unwrap_or(0);
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}.{name}")
        };
        let Some(type_ref) = hex_attribute(member, "DW_AT_type") else {
            continue;
        };
        let size = type_size(dies, type_ref).unwrap_or(0);
        let ty = member
            .attrs
            .get("DW_AT_type")
            .and_then(|value| quoted_value(value))
            .unwrap_or_else(|| "?".to_owned());
        let nested = if resolved_type(dies, type_ref).is_some_and(|die| {
            die.tag == "structure_type" || die.tag == "union_type" || die.tag == "array_type"
        }) {
            collect_members(dies, type_ref, base_offset + offset, &path, depth + 1)
        } else {
            Vec::new()
        };
        if nested.is_empty() {
            members.push(Member {
                path,
                offset: base_offset + offset,
                size,
                ty,
            });
        } else {
            members.push(Member {
                path: path.clone(),
                offset: base_offset + offset,
                size,
                ty: ty.clone(),
            });
            members.extend(nested);
        }
    }
    members
}

fn resolved_type(dies: &[Die], offset: u64) -> Option<&Die> {
    let mut current = offset;
    for _ in 0..8 {
        let die = die_by_offset(dies, current)?;
        if die.tag == "typedef" || die.tag == "const_type" || die.tag == "volatile_type" {
            current = hex_attribute(die, "DW_AT_type")?;
        } else {
            return Some(die);
        }
    }
    None
}

fn array_info(dies: &[Die], offset: u64) -> Option<(u64, u64, String)> {
    let die = resolved_type(dies, offset)?;
    if die.tag != "array_type" {
        return None;
    }
    let element_size =
        hex_attribute(die, "DW_AT_type").and_then(|offset| type_size(dies, offset))?;
    let element_type = die
        .attrs
        .get("DW_AT_type")
        .and_then(|value| quoted_value(value))
        .unwrap_or_else(|| "?".to_owned());
    Some((array_count(dies, die.offset)?, element_size, element_type))
}

fn array_count(dies: &[Die], offset: u64) -> Option<u64> {
    let index = dies.iter().position(|die| die.offset == offset)?;
    dies[index + 1..]
        .iter()
        .take_while(|die| die.tag == "subrange_type")
        .find_map(|die| hex_attribute(die, "DW_AT_count"))
}

fn die_by_offset(dies: &[Die], offset: u64) -> Option<&Die> {
    dies.iter().find(|die| die.offset == offset)
}

fn hex_attribute(die: &Die, name: &str) -> Option<u64> {
    let value = die.attrs.get(name)?;
    let start = value.find("0x")? + 2;
    let digits = value[start..]
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect::<String>();
    u64::from_str_radix(&digits, 16).ok()
}

fn quoted(die: &Die, name: &str) -> Option<String> {
    let value = die.attrs.get(name)?;
    quoted_value(value)
}

fn quoted_value(value: &str) -> Option<String> {
    let start = value.find('"')? + 1;
    let end = value[start..].find('"')? + start;
    Some(value[start..end].to_owned())
}

fn return_location(function: &FunctionInfo) -> String {
    let ty = function.return_type.as_str();
    let size = function.return_size.unwrap_or(0);
    if ty == "void" {
        "void".to_owned()
    } else if ty.contains('*') {
        "%rax".to_owned()
    } else if ty == "complex" && size == 16 {
        "%xmm0, %xmm1".to_owned()
    } else if ty == "complex" || ty == "float" || ty == "double" {
        "%xmm0".to_owned()
    } else if size <= 4 {
        "%eax".to_owned()
    } else if size <= 8 {
        "%rax".to_owned()
    } else {
        "memory".to_owned()
    }
}

fn render_mapping(
    assembly: &str,
    function: &str,
    info: &FunctionInfo,
    result_type: &str,
) -> String {
    let moves = prologue_moves(assembly);
    let mut result = format!("    {function}:\n");
    for variable in &info.variables {
        if variable.frame_offset >= 0 {
            let members = leaf_members(variable);
            if members.is_empty() {
                writeln!(
                    result,
                    "      stack({:+}) = {}",
                    variable.frame_offset,
                    variable_label(variable)
                )
                .unwrap();
            } else {
                for member in members {
                    writeln!(
                        result,
                        "      stack({:+}) = {}",
                        variable.frame_offset + member.offset as i64,
                        component_label(variable, member)
                    )
                    .unwrap();
                }
            }
            continue;
        }
        let registers = entry_registers(variable, &moves);
        let parts = entry_parts(variable, registers.len());
        for (register, part) in registers.into_iter().zip(parts) {
            writeln!(result, "      {register} = {part}").unwrap();
        }
    }
    let return_registers = return_location(info)
        .split(',')
        .map(|register| register.trim().trim_start_matches('%').to_owned())
        .filter(|register| !register.is_empty() && register != "void")
        .collect::<Vec<_>>();
    if !return_registers.is_empty() {
        writeln!(
            result,
            "      return:{result_type} = {}",
            return_registers.join(", ")
        )
        .unwrap();
    }
    result
}

fn entry_registers(variable: &Variable, moves: &[Move]) -> Vec<String> {
    let end = variable.frame_offset + variable.size.unwrap_or(1) as i64;
    let mut registers = Vec::new();
    for (index, instruction) in moves.iter().enumerate() {
        let Operand::Memory(offset) = instruction.destination else {
            continue;
        };
        if offset >= variable.frame_offset && offset < end {
            resolve_source(&instruction.source, index, moves, 0, &mut registers);
        }
    }
    registers
}

fn entry_parts(variable: &Variable, count: usize) -> Vec<String> {
    if count == 0 {
        return Vec::new();
    }
    let members = leaf_members(variable);
    if count == 1 {
        if !members.is_empty() {
            return vec![
                members
                    .iter()
                    .map(|member| component_label(variable, member))
                    .collect::<Vec<_>>()
                    .join(", "),
            ];
        }
        return vec![variable_label(variable)];
    }

    let total = variable.size.unwrap_or(0);
    let mut result = Vec::new();
    for index in 0..count {
        let start = (index * 8) as u64;
        let end = if index + 1 == count {
            total
        } else {
            ((index + 1) * 8) as u64
        };
        let mut group = members
            .iter()
            .filter(|member| member.offset >= start && member.offset + member.size <= end)
            .map(|member| component_label(variable, member))
            .collect::<Vec<_>>();
        if group.is_empty() {
            group.push(format!(
                "{}.part{index}:{}",
                fragment_base(variable),
                variable_type(variable)
            ));
        }
        result.push(group.join(", "));
    }
    result
}

fn variable_label(variable: &Variable) -> String {
    format!("{}:{}", variable.name, variable_type(variable))
}

fn component_label(variable: &Variable, member: &Member) -> String {
    format!("{}.{}:{}", fragment_base(variable), member.path, member.ty)
}

fn fragment_base(variable: &Variable) -> &str {
    if variable.ty == "complex" {
        &variable.name
    } else {
        &variable.ty
    }
}

fn variable_type(variable: &Variable) -> String {
    if variable.ty == "complex" {
        if variable.size == Some(8) {
            "_Complex float".to_owned()
        } else {
            "_Complex double".to_owned()
        }
    } else {
        variable.ty.clone()
    }
}

fn leaf_members(variable: &Variable) -> Vec<&Member> {
    let mut members = variable.members.iter().collect::<Vec<_>>();
    members.sort_by_key(|member| (member.offset, member.size));
    members
        .iter()
        .filter(|member| {
            !members.iter().any(|other| {
                other.offset >= member.offset
                    && other.offset + other.size <= member.offset + member.size
                    && other.size < member.size
            })
        })
        .copied()
        .collect()
}

fn prologue_moves(assembly: &str) -> Vec<Move> {
    let mut moves = Vec::new();
    for line in assembly.lines() {
        if line.starts_with("\t.loc") && line.contains("prologue_end") {
            break;
        }
        let code = line.split('#').next().unwrap_or("").trim();
        let mut parts = code.split_whitespace();
        let Some(opcode) = parts.next() else {
            continue;
        };
        if !opcode.starts_with("mov") {
            continue;
        }
        let source = parse_operand(parts.next().unwrap_or(""));
        let destination = parse_operand(parts.next().unwrap_or(""));
        moves.push(Move {
            source,
            destination,
        });
    }
    moves
}

fn parse_operand(value: &str) -> Operand {
    let value = value.trim_end_matches(',');
    if let Some(register) = value.strip_prefix('%') {
        Operand::Register(register.to_owned())
    } else if let Some(offset) = value
        .strip_suffix("(%rbp)")
        .and_then(|offset| offset.parse().ok())
    {
        Operand::Memory(offset)
    } else {
        Operand::Other
    }
}

fn resolve_source(
    source: &Operand,
    before: usize,
    moves: &[Move],
    depth: usize,
    registers: &mut Vec<String>,
) {
    if depth >= 8 {
        return;
    }
    match source {
        Operand::Register(register) => {
            let definition = moves[..before]
                .iter()
                .enumerate()
                .rev()
                .find(|(_, instruction)| {
                    matches!(
                        &instruction.destination,
                        Operand::Register(destination) if destination == register
                    )
                });
            if let Some((index, instruction)) = definition {
                resolve_source(&instruction.source, index, moves, depth + 1, registers);
            } else if !registers.contains(register) {
                registers.push(register.clone());
            }
        }
        Operand::Memory(offset) => {
            let definition = moves[..before].iter().enumerate().rev().find(|(_, instruction)| {
                matches!(instruction.destination, Operand::Memory(destination) if destination == *offset)
            });
            if let Some((index, instruction)) = definition {
                resolve_source(&instruction.source, index, moves, depth + 1, registers);
            }
        }
        Operand::Other => {}
    }
}
