// AGENT SAFETY: 自动生成测试框架无需人类理解

include!("../src/lib.rs");
mod debug_info;

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
    if text.is_empty() {
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
    let ctype = if label == "int" {
        CType::Int
    } else {
        CType::Unsupported
    };
    Ok(ArgType { label, ctype })
}

fn planner_args(case: &Case) -> Vec<CType> {
    case.args.iter().map(|arg| arg.ctype).collect()
}

fn all_integer(case: &Case) -> bool {
    matches!(case.result.ctype, CType::Int)
        && case.args.iter().all(|arg| matches!(arg.ctype, CType::Int))
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
    let planner_args = planner_args(case);
    let mut raw_plan = MaybeUninit::<Plan>::uninit();
    let args = if planner_args.is_empty() {
        std::ptr::null()
    } else {
        planner_args.as_ptr()
    };
    // SAFETY: `args` points to `case.args.len()` initialized values and
    // `raw_plan` points to writable storage of the required type.
    let status = unsafe {
        get_plan(
            args,
            planner_args.len() as c_int,
            case.result.ctype,
            raw_plan.as_mut_ptr(),
        )
    };
    let get_plan_status = error_name(&status);
    let returned_plan = match status {
        AbiPlanError::Ok => {
            // SAFETY: get_plan returned Ok only after writing the whole Plan.
            Some(unsafe { raw_plan.assume_init() })
        }
        _ => None,
    };

    let Some(plan) = returned_plan else {
        return Err(failure_report(
            case,
            get_plan_status,
            None,
            &case.expected,
            "no executable call plan",
            None,
            &debug,
            None,
        ));
    };

    let expected = match case.expected.parse::<i32>() {
        Ok(expected) if all_integer(case) => expected,
        _ => {
            return Err(failure_report(
                case,
                get_plan_status,
                Some(&plan),
                &case.expected,
                "a supported scalar result",
                None,
                &debug,
                None,
            ));
        }
    };
    let inputs = match parse_int_inputs(case) {
        Ok(inputs) => inputs,
        Err(error) => {
            return Err(failure_report(
                case,
                get_plan_status,
                Some(&plan),
                &case.expected,
                &error,
                None,
                &debug,
                None,
            ));
        }
    };
    let harness = match render_harness(case, &plan, &inputs) {
        Ok(harness) => harness,
        Err(error) => {
            return Err(failure_report(
                case,
                get_plan_status,
                Some(&plan),
                &case.expected,
                &error,
                None,
                &debug,
                None,
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
            get_plan_status,
            Some(&plan),
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
            get_plan_status,
            Some(&plan),
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
    if execution.stdout.len() != 4 {
        return Err(failure_report(
            case,
            get_plan_status,
            Some(&plan),
            &case.expected,
            "generated binary did not write one 32-bit result",
            Some(&harness),
            &debug,
            None,
        ));
    }
    let actual = i32::from_le_bytes(execution.stdout[..4].try_into().unwrap());
    if actual != expected {
        return Err(failure_report(
            case,
            get_plan_status,
            Some(&plan),
            &case.expected,
            &actual.to_string(),
            Some(&harness),
            &debug,
            None,
        ));
    }

    println!("PASS {}", case.name);
    Ok(())
}

fn parse_int_inputs(case: &Case) -> Result<Vec<i32>, String> {
    if !all_integer(case) {
        return Err("the generated harness only supports integer scalar calls".to_owned());
    }
    if case.inputs.is_empty() {
        return Ok(Vec::new());
    }
    let inputs = case
        .inputs
        .split(',')
        .map(|input| {
            input
                .trim()
                .parse::<i32>()
                .map_err(|error| format!("invalid integer input `{input}`: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if inputs.len() != case.args.len() {
        return Err(format!(
            "signature has {} argument(s), input has {}",
            case.args.len(),
            inputs.len()
        ));
    }
    Ok(inputs)
}

fn render_harness(case: &Case, plan: &Plan, inputs: &[i32]) -> Result<String, String> {
    let mut asm = String::new();
    for (index, input) in inputs.iter().enumerate() {
        let register = gpr32(plan.args[index])
            .ok_or_else(|| format!("argument {index} has no register in the returned plan"))?;
        let register64 = gpr64(plan.args[index]).unwrap();
        writeln!(
            asm,
            "        \"# arg[{index}] {} = {input} -> %{register64}\\n\\t\"",
            case.args[index].label
        )
        .unwrap();
        writeln!(asm, "        \"movl ${input}, %{register}\\n\\t\"").unwrap();
    }
    let result = gpr32(plan.result)
        .ok_or_else(|| "return value has no register in the returned plan".to_owned())?;
    let result64 = gpr64(plan.result).unwrap();
    writeln!(
        asm,
        "        \"# result {} <- %{result64}\\n\\t\"",
        case.result.label
    )
    .unwrap();
    writeln!(asm, "        \"call {}\\n\\t\"", case.name).unwrap();
    writeln!(asm, "        \"movl %{result}, %r8d\\n\\t\"").unwrap();
    asm.push_str(
        "        \"subq $16, %rsp\\n\\t\"\n\
         \x20       \"movl %r8d, (%rsp)\\n\\t\"\n\
         \x20       \"movl $1, %eax\\n\\t\"\n\
         \x20       \"movl $1, %edi\\n\\t\"\n\
         \x20       \"movq %rsp, %rsi\\n\\t\"\n\
         \x20       \"movl $4, %edx\\n\\t\"\n\
         \x20       \"syscall\\n\\t\"\n\
         \x20       \"addq $16, %rsp\\n\\t\"\n\
         \x20       \"movl $60, %eax\\n\\t\"\n\
         \x20       \"xorl %edi, %edi\\n\\t\"\n\
         \x20       \"syscall\\n\\t\"\n\
         \x20       \"ud2\\n\\t\"\n",
    );
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

fn gpr32(register: Register) -> Option<&'static str> {
    match register {
        Register::Rdi => Some("edi"),
        Register::Rax => Some("eax"),
        Register::None => None,
    }
}

fn gpr64(register: Register) -> Option<&'static str> {
    match register {
        Register::Rdi => Some("rdi"),
        Register::Rax => Some("rax"),
        Register::None => None,
    }
}

fn error_name(error: &AbiPlanError) -> &'static str {
    match error {
        AbiPlanError::Ok => "Ok",
        AbiPlanError::NotSupport => "NotSupport",
        AbiPlanError::InvalidArgument => "InvalidArgument",
    }
}

fn failure_report(
    case: &Case,
    get_plan_status: &str,
    plan: Option<&Plan>,
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
        let inputs = parse_int_inputs(case).ok();
        for (index, arg) in case.args.iter().enumerate() {
            let value = inputs
                .as_ref()
                .and_then(|inputs| inputs.get(index))
                .map_or_else(|| "?".to_owned(), i32::to_string);
            let register = gpr64(plan.args[index])
                .map_or_else(|| "<none>".to_owned(), |register| format!("%{register}"));
            writeln!(
                report,
                "    arg[{index}] {} = {value} -> {register}",
                arg.label
            )
            .unwrap();
        }
        let result = gpr64(plan.result)
            .map_or_else(|| "<none>".to_owned(), |register| format!("%{register}"));
        writeln!(report, "    result {} <- {result}", case.result.label).unwrap();
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
