use core::slice;
use std::ffi::c_int;

#[repr(C)]
enum AbiPlanError {
    Ok = 0,
    NotSupport = -1,
    InvalidArgument = -2,
}

#[derive(Clone, Copy)]
#[repr(u8)]
enum CType {
    // 增加Int128时，需要修改很多地方！
    Int,
    Unsupported,
    Void,
}

#[derive(Clone, Copy)]
#[repr(u8)]
enum Register {
    None,
    Rdi,
    Rsi,
    Rdx,
    Rcx,
    R8,
    R9,
    Rax,
}

#[repr(C)]
struct Plan {
    args: [Register; 6],
    result: Register,
}

const X86_64_GPR_LEN: usize = 6;


//生成简单情况寄存器列表
fn gen_simple_regs(count: usize) -> [Register; X86_64_GPR_LEN] {
    const ARG_REGISTERS: [Register; X86_64_GPR_LEN] = [
        Register::Rdi,
        Register::Rsi,
        Register::Rdx,
        Register::Rcx,
        Register::R8,
        Register::R9,
    ];

    let mut registers = [Register::None; X86_64_GPR_LEN];
    for (register, argument_register) in registers.iter_mut().zip(ARG_REGISTERS).take(count) {
        *register = argument_register;
    }
    registers
}
// 检查是否是简单情况(gpr size以内，都是标量/指针)
fn is_simple_signature(args: &[CType], result: CType) -> bool {
    args.len() < X86_64_GPR_LEN
    //后续还要加入指针等类型
        && (args.is_empty() || args.iter().all(|arg| matches!(arg, CType::Int)))
        && matches!(result, CType::Int | CType::Void)
}

fn do_plan_simple(args: &[CType], result: CType) -> Result<Plan, AbiPlanError> {
    debug_assert!(is_simple_signature(args, result));
    let result = match result {
        CType::Int => Register::Rax,
        CType::Void => Register::None,
        CType::Unsupported => return Err(AbiPlanError::NotSupport),
    };
    Ok(Plan {
        args: gen_simple_regs(args.len()),
        result,
    })
}

fn do_plan(args: &[CType], result: CType) -> Result<Plan, AbiPlanError> {
    if is_simple_signature(args, result){
    do_plan_simple(args, result)
    }else{
        return Err(AbiPlanError::NotSupport)
    }
}

/// # Safety
/// plan 必须指向可写的 Plan。
unsafe extern "C" fn get_plan(
    args: *const CType,
    len: c_int,
    result: CType,
    plan: *mut Plan,
) -> AbiPlanError {
    if len < 0 || plan.is_null() || (len > 0 && args.is_null()) {
        return AbiPlanError::InvalidArgument;
    }
    // 传入空args视作fn(void)
    let args = if len == 0 {
        &[]
    } else {
        // SAFETY: 已检查len大于0且args非空
        unsafe { slice::from_raw_parts(args, len as usize) }
    };

    match do_plan(args, result) {
        Ok(p) => {
            // SAFETY: plan已检查非NULL，由调用方确保有效
            unsafe { plan.write(p) };
            AbiPlanError::Ok
        }
        Err(e) => e,
    }
}
