// AGENT SAFETY: 自动生成测试框架无需人类理解

mod debug_info;

use abi_plan::{free_plan, get_plan, AbiPlanError, Allocation, CFunc, CallPlan, CType, Place, Register};
use std::{
    collections::BTreeMap,
    env,
    ffi::OsStr,
    fmt::Write as _,
    fs,
    mem::MaybeUninit,
    path::{Path, PathBuf},
    process::Command,
};

use debug_info::DebugReport;

struct Case {
    name: String,
    source: PathBuf,
    args: Vec<ArgType>,
    inputs: String,
    result: ArgType,
    expected: String,
}

struct ArgType {
    label: String,
    ctype: CType,
    kind: TypeKind,
}

/// 测试自身的类型语义：决定如何解析输入/期望与生成汇编。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeKind {
    Int { len: usize },
    Float,
    Double,
    ComplexFloat,
    ComplexDouble,
    Void,
    Unsupported,
}

enum Input {
    Int(i64),
    Float(f32),
    Double(f64),
    ComplexF32 { re: f32, im: f32 },
    ComplexF64 { re: f64, im: f64 },
    Address(i64),
}

enum Expected {
    Int(i32),
    Float(f32),
    Double(f64),
    ComplexF32 { re: f32, im: f32 },
    ComplexF64 { re: f64, im: f64 },
}

/// 拥有 `CallPlan` 的守卫，drop 时统一调用 `free_plan`。
struct OwnedPlan(Option<CallPlan>);

impl Drop for OwnedPlan {
    fn drop(&mut self) {
        if let Some(plan) = self.0.take() {
            // SAFETY: 本守卫独占该 CallPlan，且只释放一次。
            unsafe { free_plan(plan) };
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let root = option_env!("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            let executable = env::current_exe().ok()?;
            executable
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        })
        .ok_or("cannot locate repository root")?;
    let cases_dir = root.join("test/cases");
    let work_dir = root
        .join("target/abi-plan-tests")
        .join(std::process::id().to_string());
    fs::create_dir_all(&work_dir).map_err(|error| error.to_string())?;

    let mut sources = fs::read_dir(&cases_dir)
        .map_err(|error| format!("{}: {error}", cases_dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension() == Some(OsStr::new("c")))
        .collect::<Vec<_>>();
    sources.sort();
    if sources.is_empty() {
        return Err(format!("no C cases found in {}", cases_dir.display()));
    }

    let mut failed = 0;
    let mut passed = 0;
    let mut failures = Vec::new();
    for source in sources {
        let result = parse_case(&source).and_then(|case| run_case(&case, &work_dir));
        match result {
            Ok(()) => passed += 1,
            Err(error) => {
                eprintln!("{}", error.lines().next().unwrap_or("FAIL unknown case"));
                failures.push(error);
                failed += 1;
            }
        }
    }

    let _ = fs::remove_dir_all(&work_dir);
    let report_path = root.join("target/test-report.txt");
    let mut report = format!("pass: {passed}\nfail: {failed}\n\n");
    for failure in &failures {
        report.push_str(failure);
        report.push('\n');
    }
    fs::write(&report_path, report).map_err(|error| error.to_string())?;
    println!("report: {}", report_path.display());

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{failed} test(s) failed"))
    }
}

fn parse_case(source: &Path) -> Result<Case, String> {
    let text =
        fs::read_to_string(source).map_err(|error| format!("{}: {error}", source.display()))?;
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(comment) = line.strip_prefix("//") else {
            continue;
        };
        let Some((key, value)) = comment.split_once(':') else {
            continue;
        };
        fields.insert(key.trim(), value.trim());
    }

    let args = parse_types(required(&fields, "args", source)?, source)?;
    let result = parse_type(fields.get("result").copied().unwrap_or("int"), source)?;
    let inputs = required(&fields, "input", source)?.to_owned();
    let expected = required(&fields, "expected", source)?.to_owned();

    let name = source
        .file_stem()
        .and_then(OsStr::to_str)
        .ok_or_else(|| format!("{}: invalid file name", source.display()))?
        .to_owned();
    Ok(Case {
        name,
        source: source.to_owned(),
        args,
        inputs,
        result,
        expected,
    })
}

fn required<'a>(
    fields: &'a BTreeMap<&str, &str>,
    key: &str,
    source: &Path,
) -> Result<&'a str, String> {
    fields
        .get(key)
        .copied()
        .ok_or_else(|| format!("{}: missing `// {key}: ...`", source.display()))
}

fn parse_types(text: &str, source: &Path) -> Result<Vec<ArgType>, String> {
    if text.is_empty() || text == "void" {
        return Ok(Vec::new());
    }
    text.split(',')
        .map(|item| parse_type(item, source))
        .collect()
}

fn parse_type(text: &str, source: &Path) -> Result<ArgType, String> {
    let label = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if label.is_empty() {
        return Err(format!("{}: empty C type", source.display()));
    }
    let (kind, ctype) = match label.as_str() {
        "int" => (TypeKind::Int { len: 4 }, CType::Int { len: 4, is_signed: true }),
        "void" => (TypeKind::Void, CType::Void),
        "float" => (TypeKind::Float, CType::Float { len: 4 }),
        "double" => (TypeKind::Double, CType::Float { len: 8 }),
        "_Complex float" => (TypeKind::ComplexFloat, CType::Float { len: 8 }),
        "_Complex double" => (TypeKind::ComplexDouble, CType::Float { len: 16 }),
        // 指针按注释用 Int{len:8} 表示。
        _ if label.ends_with('*') => (TypeKind::Int { len: 8 }, CType::Int { len: 8, is_signed: false }),
        // 聚合体：构造占位，交给 get_plan 返回 Unsupported。
        _ => (TypeKind::Unsupported, CType::Struct { ptr: std::ptr::null_mut(), len: 0 }),
    };
    Ok(ArgType { label, ctype, kind })
}

fn parse_inputs(case: &Case) -> Result<Vec<Input>, String> {
    if case.args.is_empty() {
        return if case.inputs.trim().is_empty() {
            Ok(Vec::new())
        } else {
            Err("unexpected input for empty signature".to_owned())
        };
    }
    let parts: Vec<&str> = case.inputs.split(',').map(str::trim).collect();
    if parts.len() != case.args.len() {
        return Err(format!(
            "signature has {} argument(s), input has {}",
            case.args.len(),
            parts.len()
        ));
    }
    parts
        .iter()
        .zip(&case.args)
        .map(|(text, arg)| parse_input(arg, text))
        .collect()
}

fn parse_input(arg: &ArgType, text: &str) -> Result<Input, String> {
    match arg.kind {
        TypeKind::Int { .. } => {
            if let Some(rest) = text.strip_prefix('&') {
                let value = rest
                    .trim()
                    .parse::<i64>()
                    .map_err(|error| format!("invalid address input `{text}`: {error}"))?;
                Ok(Input::Address(value))
            } else {
                let value = text
                    .parse::<i64>()
                    .map_err(|error| format!("invalid integer input `{text}`: {error}"))?;
                Ok(Input::Int(value))
            }
        }
        TypeKind::Float => text
            .parse::<f32>()
            .map(Input::Float)
            .map_err(|error| format!("invalid float input `{text}`: {error}")),
        TypeKind::Double => text
            .parse::<f64>()
            .map(Input::Double)
            .map_err(|error| format!("invalid double input `{text}`: {error}")),
        TypeKind::ComplexFloat => {
            let (re, im) = split_complex(text)?;
            Ok(Input::ComplexF32 { re: re as f32, im: im as f32 })
        }
        TypeKind::ComplexDouble => {
            let (re, im) = split_complex(text)?;
            Ok(Input::ComplexF64 { re, im })
        }
        TypeKind::Void => Err("void argument cannot have input".to_owned()),
        TypeKind::Unsupported => Err(format!("unsupported argument type `{}`", arg.label)),
    }
}

/// 解析 `"3.0 + 4.0i"` 形式的复数，返回 `(re, im)`。
fn split_complex(text: &str) -> Result<(f64, f64), String> {
    let text = text.trim();
    let without_i = text
        .strip_suffix('i')
        .ok_or_else(|| format!("invalid complex input `{text}`"))?;
    let Some(pos) = without_i.rfind(|c: char| c == '+' || c == '-') else {
        return Err(format!("invalid complex input `{text}`"));
    };
    if pos == 0 {
        return Err(format!("invalid complex input `{text}`"));
    }
    let sign = if &without_i[pos..pos + 1] == "-" { -1.0 } else { 1.0 };
    let re = without_i[..pos]
        .trim()
        .parse::<f64>()
        .map_err(|error| format!("invalid complex input `{text}`: {error}"))?;
    let im = without_i[pos + 1..]
        .trim()
        .parse::<f64>()
        .map_err(|error| format!("invalid complex input `{text}`: {error}"))?;
    Ok((re, sign * im))
}

fn parse_expected(case: &Case) -> Result<Expected, String> {
    match case.result.kind {
        TypeKind::Int { .. } => case
            .expected
            .trim()
            .parse::<i32>()
            .map(Expected::Int)
            .map_err(|error| format!("invalid integer expected `{}`: {error}", case.expected)),
        TypeKind::Float => case
            .expected
            .trim()
            .parse::<f32>()
            .map(Expected::Float)
            .map_err(|error| format!("invalid float expected `{}`: {error}", case.expected)),
        TypeKind::Double => case
            .expected
            .trim()
            .parse::<f64>()
            .map(Expected::Double)
            .map_err(|error| format!("invalid double expected `{}`: {error}", case.expected)),
        TypeKind::ComplexFloat => {
            let (re, im) = split_complex(&case.expected)?;
            Ok(Expected::ComplexF32 { re: re as f32, im: im as f32 })
        }
        TypeKind::ComplexDouble => {
            let (re, im) = split_complex(&case.expected)?;
            Ok(Expected::ComplexF64 { re, im })
        }
        TypeKind::Void | TypeKind::Unsupported => {
            Err(format!("unsupported result type `{}`", case.result.label))
        }
    }
}

fn result_bytes(kind: TypeKind) -> usize {
    match kind {
        TypeKind::Int { .. } | TypeKind::Float => 4,
        TypeKind::Double | TypeKind::ComplexFloat => 8,
        TypeKind::ComplexDouble => 16,
        TypeKind::Void | TypeKind::Unsupported => 0,
    }
}

fn run_case(case: &Case, work_dir: &Path) -> Result<(), String> {
    let clang = clang();
    let debug = debug_info::render(
        &clang,
        &case.source,
        &case.name,
        &case.result.label,
        work_dir,
    )
    .unwrap_or_else(|error| DebugReport {
        mapping: format!("    unavailable: {error}\n"),
    });

    let arg_ctypes: Vec<CType> = case.args.iter().map(|arg| arg.ctype).collect();
    let cfunc = CFunc {
        args: if arg_ctypes.is_empty() {
            std::ptr::null_mut()
        } else {
            arg_ctypes.as_ptr() as *mut CType
        },
        args_len: arg_ctypes.len() as u32,
        ret: case.result.ctype,
    };
    let mut raw_plan = MaybeUninit::<CallPlan>::uninit();
    // SAFETY: `args` 指向 `arg_ctypes.len()` 个已初始化值，`raw_plan` 指向可写存储。
    let status = unsafe { get_plan(cfunc, raw_plan.as_mut_ptr()) };
    let status_name = error_name(&status);
    if status != AbiPlanError::Ok {
        return Err(failure_report(
            case,
            status_name,
            None,
            &case.expected,
            "no executable call plan",
            None,
            &debug,
            None,
        ));
    }
    // SAFETY: get_plan 返回 Ok 后才写满 CallPlan。
    let owned_plan = OwnedPlan(Some(unsafe { raw_plan.assume_init() }));
    let plan = owned_plan.0.as_ref().expect("just initialized");

    let expected = match parse_expected(case) {
        Ok(expected) => expected,
        Err(error) => {
            return Err(failure_report(
                case, status_name, Some(plan), &case.expected, &error, None, &debug, None,
            ));
        }
    };
    let inputs = match parse_inputs(case) {
        Ok(inputs) => inputs,
        Err(error) => {
            return Err(failure_report(
                case, status_name, Some(plan), &case.expected, &error, None, &debug, None,
            ));
        }
    };
    let harness = match render_harness(case, plan, &inputs) {
        Ok(harness) => harness,
        Err(error) => {
            return Err(failure_report(
                case, status_name, Some(plan), &case.expected, &error, None, &debug, None,
            ));
        }
    };

    let case_dir = work_dir.join(&case.name);
    fs::create_dir_all(&case_dir).map_err(|error| error.to_string())?;
    let harness_path = case_dir.join("harness.c");
    let binary_path = case_dir.join("harness");
    fs::write(&harness_path, &harness).map_err(|error| error.to_string())?;

    let compile = Command::new(&clang)
        .args([
            "--target=x86_64-unknown-linux-gnu",
            "-O0",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-ffreestanding",
            "-fno-stack-protector",
            "-fno-pic",
            "-nostdlib",
            "-static",
            "-fuse-ld=lld",
            "-Wl,-no-pie",
            "-Wl,-e,_start",
        ])
        .arg(&case.source)
        .arg(&harness_path)
        .arg("-o")
        .arg(&binary_path)
        .output()
        .map_err(|error| format!("cannot run {clang}: {error}"))?;
    if !compile.status.success() {
        return Err(failure_report(
            case,
            status_name,
            Some(plan),
            &case.expected,
            "clang rejected the generated call",
            Some(&harness),
            &debug,
            Some(&String::from_utf8_lossy(&compile.stderr)),
        ));
    }

    let mut runner = if env::consts::ARCH == "x86_64" {
        Command::new(&binary_path)
    } else {
        let qemu = ["qemu-x86_64-static", "qemu-x86_64"]
            .into_iter()
            .find(|name| Command::new(name).arg("--version").output().is_ok())
            .ok_or("cannot find qemu-x86_64 for the x86_64 test binary")?;
        let mut command = Command::new(qemu);
        command.arg(&binary_path);
        command
    };
    let execution = runner
        .output()
        .map_err(|error| format!("cannot execute test binary: {error}"))?;
    if !execution.status.success() {
        return Err(failure_report(
            case,
            status_name,
            Some(plan),
            &case.expected,
            &format!(
                "generated binary exited with {}: {}",
                execution.status,
                String::from_utf8_lossy(&execution.stderr).trim()
            ),
            Some(&harness),
            &debug,
            None,
        ));
    }
    if let Err(error) = check_result(case.result.kind, &expected, &execution.stdout) {
        return Err(failure_report(
            case, status_name, Some(plan), &case.expected, &error, Some(&harness), &debug, None,
        ));
    }

    println!("PASS {}", case.name);
    Ok(())
}

fn check_result(kind: TypeKind, expected: &Expected, stdout: &[u8]) -> Result<(), String> {
    let need = result_bytes(kind);
    if stdout.len() != need {
        return Err(format!("generated binary wrote {} bytes, expected {need}", stdout.len()));
    }
    match (kind, expected) {
        (TypeKind::Int { .. }, Expected::Int(expected)) => {
            let got = i32::from_le_bytes(stdout[..4].try_into().unwrap());
            if got == *expected {
                Ok(())
            } else {
                Err(format!("expected {expected}, got {got}"))
            }
        }
        (TypeKind::Float, Expected::Float(expected)) => {
            let got = f32::from_le_bytes(stdout[..4].try_into().unwrap());
            if got.to_bits() == expected.to_bits() {
                Ok(())
            } else {
                Err(format!("expected {expected}, got {got}"))
            }
        }
        (TypeKind::Double, Expected::Double(expected)) => {
            let got = f64::from_le_bytes(stdout[..8].try_into().unwrap());
            if got.to_bits() == expected.to_bits() {
                Ok(())
            } else {
                Err(format!("expected {expected}, got {got}"))
            }
        }
        (TypeKind::ComplexFloat, Expected::ComplexF32 { re, im }) => {
            let got_re = f32::from_le_bytes(stdout[..4].try_into().unwrap());
            let got_im = f32::from_le_bytes(stdout[4..8].try_into().unwrap());
            if got_re.to_bits() == re.to_bits() && got_im.to_bits() == im.to_bits() {
                Ok(())
            } else {
                Err(format!("expected {re} + {im}i, got {got_re} + {got_im}i"))
            }
        }
        (TypeKind::ComplexDouble, Expected::ComplexF64 { re, im }) => {
            let got_re = f64::from_le_bytes(stdout[..8].try_into().unwrap());
            let got_im = f64::from_le_bytes(stdout[8..16].try_into().unwrap());
            if got_re.to_bits() == re.to_bits() && got_im.to_bits() == im.to_bits() {
                Ok(())
            } else {
                Err(format!("expected {re} + {im}i, got {got_re} + {got_im}i"))
            }
        }
        _ => Err("unsupported result type".to_owned()),
    }
}

fn align_up(value: usize, align: usize) -> usize {
    value.div_ceil(align) * align
}

fn emit(asm: &mut String, instruction: &str) {
    writeln!(asm, "        \"{instruction}\\n\\t\"").expect("write to String cannot fail");
}

fn place_regs(place: &Place) -> &[Register] {
    match place.alloc {
        Allocation::Reg { reg, len } => {
            // SAFETY: Reg 中的指针由 Place::reg 分配，len 为其长度。
            unsafe { std::slice::from_raw_parts(reg, len) }
        }
        _ => &[],
    }
}

fn render_harness(case: &Case, plan: &CallPlan, inputs: &[Input]) -> Result<String, String> {
    let args = plan.args();
    if args.len() != inputs.len() {
        return Err(format!(
            "plan has {} argument(s), expected {}",
            args.len(),
            inputs.len()
        ));
    }

    let pointer_count = inputs.iter().filter(|i| matches!(i, Input::Address(_))).count();
    let ptr_space = align_up(pointer_count * 8, 16);
    let frame = plan.stack_size + ptr_space;

    let mut asm = String::new();
    if frame > 0 {
        emit(&mut asm, &format!("subq ${frame}, %rsp"));
    }

    // 栈参数写入（offset 相对 call 前的 %rsp）。
    for (index, place) in args.iter().enumerate() {
        if let Allocation::Stack { offset } = place.alloc {
            match &inputs[index] {
                Input::Int(value) => emit(&mut asm, &format!("movl ${}, {offset}(%rsp)", *value as i32)),
                _ => return Err(format!("argument {index}: non-integer stack argument is not supported")),
            }
        }
    }

    // 寄存器参数加载。
    let mut ptr_slot = 0usize;
    for (index, place) in args.iter().enumerate() {
        if !matches!(place.alloc, Allocation::Reg { .. }) {
            continue;
        }
        let regs = place_regs(place);
        let reg0 = *regs
            .first()
            .ok_or_else(|| format!("argument {index} has no register in the returned plan"))?;
        match &inputs[index] {
            Input::Int(value) => {
                let gpr = gpr32_name(reg0)
                    .ok_or_else(|| format!("argument {index} has no integer register"))?;
                emit(&mut asm, &format!("movl ${}, %{gpr}", *value as i32));
            }
            Input::Float(value) => {
                let xmm = xmm_name(reg0)
                    .ok_or_else(|| format!("argument {index} has no SSE register"))?;
                emit(&mut asm, &format!("movl ${}, %r10d", value.to_bits()));
                emit(&mut asm, &format!("movd %r10d, %{xmm}"));
            }
            Input::Double(value) => {
                let xmm = xmm_name(reg0)
                    .ok_or_else(|| format!("argument {index} has no SSE register"))?;
                emit(&mut asm, &format!("movabs ${}, %r10", value.to_bits()));
                emit(&mut asm, &format!("movq %r10, %{xmm}"));
            }
            Input::ComplexF32 { re, im } => {
                let xmm = xmm_name(reg0)
                    .ok_or_else(|| format!("argument {index} has no SSE register"))?;
                let bits = ((im.to_bits() as u64) << 32) | (re.to_bits() as u64);
                emit(&mut asm, &format!("movabs ${bits}, %r10"));
                emit(&mut asm, &format!("movq %r10, %{xmm}"));
            }
            Input::ComplexF64 { re, im } => {
                let xmm0 = xmm_name(reg0)
                    .ok_or_else(|| format!("argument {index} has no SSE register"))?;
                let reg1 = *regs
                    .get(1)
                    .ok_or_else(|| format!("argument {index} has no second SSE register"))?;
                let xmm1 = xmm_name(reg1)
                    .ok_or_else(|| format!("argument {index} has no second SSE register"))?;
                emit(&mut asm, &format!("movabs ${}, %r10", re.to_bits()));
                emit(&mut asm, &format!("movq %r10, %{xmm0}"));
                emit(&mut asm, &format!("movabs ${}, %r10", im.to_bits()));
                emit(&mut asm, &format!("movq %r10, %{xmm1}"));
            }
            Input::Address(value) => {
                let gpr = register_name(reg0);
                let addr = plan.stack_size + ptr_slot * 8;
                emit(&mut asm, &format!("movq ${value}, {addr}(%rsp)"));
                emit(&mut asm, &format!("leaq {addr}(%rsp), %{gpr}"));
                ptr_slot += 1;
            }
        }
    }

    emit(&mut asm, &format!("call {}", case.name));

    if frame > 0 {
        emit(&mut asm, &format!("addq ${frame}, %rsp"));
    }

    // 返回值写出到输出缓冲。
    emit(&mut asm, "subq $16, %rsp");
    match case.result.kind {
        TypeKind::Int { .. } => emit(&mut asm, "movl %eax, (%rsp)"),
        TypeKind::Float => emit(&mut asm, "movd %xmm0, (%rsp)"),
        TypeKind::Double => emit(&mut asm, "movq %xmm0, (%rsp)"),
        TypeKind::ComplexFloat => emit(&mut asm, "movq %xmm0, (%rsp)"),
        TypeKind::ComplexDouble => {
            emit(&mut asm, "movq %xmm0, (%rsp)");
            emit(&mut asm, "movq %xmm1, 8(%rsp)");
        }
        TypeKind::Void | TypeKind::Unsupported => {
            return Err(format!("unsupported result type `{}`", case.result.label));
        }
    }
    emit(&mut asm, "movl $1, %eax");
    emit(&mut asm, "movl $1, %edi");
    emit(&mut asm, "movq %rsp, %rsi");
    emit(&mut asm, &format!("movl ${}, %edx", result_bytes(case.result.kind)));
    emit(&mut asm, "syscall");
    emit(&mut asm, "addq $16, %rsp");
    emit(&mut asm, "movl $60, %eax");
    emit(&mut asm, "xorl %edi, %edi");
    emit(&mut asm, "syscall");
    emit(&mut asm, "ud2");

    Ok(format!(
        "__attribute__((naked, noreturn))\n\
         void _start(void) {{\n\
         \x20   __asm__ volatile(\n\
         {asm}\
         \x20   );\n\
         }}\n"
    ))
}

fn clang() -> String {
    env::var("CLANG").unwrap_or_else(|_| "clang".to_owned())
}

fn register_name(register: Register) -> &'static str {
    match register {
        Register::Rax => "rax",
        Register::Rcx => "rcx",
        Register::Rdx => "rdx",
        Register::Rbx => "rbx",
        Register::Rsp => "rsp",
        Register::Rbp => "rbp",
        Register::Rsi => "rsi",
        Register::Rdi => "rdi",
        Register::R8 => "r8",
        Register::R9 => "r9",
        Register::Xmm0 => "xmm0",
        Register::Xmm1 => "xmm1",
        Register::Xmm2 => "xmm2",
        Register::Xmm3 => "xmm3",
        Register::Xmm4 => "xmm4",
        Register::Xmm5 => "xmm5",
        Register::Xmm6 => "xmm6",
        Register::Xmm7 => "xmm7",
    }
}

fn gpr32_name(register: Register) -> Option<&'static str> {
    match register {
        Register::Rdi => Some("edi"),
        Register::Rsi => Some("esi"),
        Register::Rdx => Some("edx"),
        Register::Rcx => Some("ecx"),
        Register::R8 => Some("r8d"),
        Register::R9 => Some("r9d"),
        Register::Rax => Some("eax"),
        _ => None,
    }
}

fn xmm_name(register: Register) -> Option<&'static str> {
    match register {
        Register::Xmm0 => Some("xmm0"),
        Register::Xmm1 => Some("xmm1"),
        Register::Xmm2 => Some("xmm2"),
        Register::Xmm3 => Some("xmm3"),
        Register::Xmm4 => Some("xmm4"),
        Register::Xmm5 => Some("xmm5"),
        Register::Xmm6 => Some("xmm6"),
        Register::Xmm7 => Some("xmm7"),
        _ => None,
    }
}

fn error_name(error: &AbiPlanError) -> &'static str {
    match error {
        AbiPlanError::Ok => "Ok",
        AbiPlanError::Unsupported => "Unsupported",
        AbiPlanError::InvalidArgument => "InvalidArgument",
    }
}

fn place_label(place: &Place) -> String {
    match place.alloc {
        Allocation::Reg { reg, len } => {
            // SAFETY: Reg 中的指针由 Place::reg 分配，len 为其长度。
            let regs = unsafe { std::slice::from_raw_parts(reg, len) };
            regs.iter()
                .map(|register| format!("%{}", register_name(*register)))
                .collect::<Vec<_>>()
                .join(", ")
        }
        Allocation::Stack { offset } => format!("stack+{offset}"),
        Allocation::Sret => "sret".to_owned(),
        Allocation::Void => "void".to_owned(),
    }
}

fn failure_report(
    case: &Case,
    get_plan_status: &str,
    plan: Option<&CallPlan>,
    expect: &str,
    got: &str,
    harness: Option<&str>,
    debug: &DebugReport,
    compiler_output: Option<&str>,
) -> String {
    let args = case
        .args
        .iter()
        .map(|arg| arg.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut report = format!(
        "FAIL {}\n  signature: ({args}) -> {}\n  input: {}\n  expect: {expect}\n  got: {got}\n",
        case.name, case.result.label, case.inputs
    );
    if let Some(plan) = plan {
        writeln!(report, "  get_plan return: Ok").unwrap();
        writeln!(report, "  get_plan register map:").unwrap();
        for (index, arg) in case.args.iter().enumerate() {
            let location = plan
                .args()
                .get(index)
                .map(place_label)
                .unwrap_or_else(|| "<none>".to_owned());
            writeln!(report, "    arg[{index}] {} -> {location}", arg.label).unwrap();
        }
        writeln!(report, "    result {} <- {}", case.result.label, place_label(&plan.ret)).unwrap();
    } else {
        writeln!(report, "  get_plan return: {get_plan_status}").unwrap();
        writeln!(report, "  get_plan register map: unavailable").unwrap();
    }
    writeln!(report, "  C data mapping:").unwrap();
    report.push_str(&debug.mapping);
    if let Some(output) = compiler_output {
        if let Some(harness) = harness {
            write!(report, "  generated harness:\n{harness}").unwrap();
        }
        write!(report, "  clang stderr:\n{output}").unwrap();
    }
    report
}
