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
    Int,
    Unsupported,
}

#[derive(Clone, Copy)]
#[repr(u8)]
enum Register {
    None,
    Rdi,
    Rax,
}

#[repr(C)]
struct Plan {
    args: [Register; 6],
    result: Register,
}

fn do_plan(args: &[CType], result: CType) -> Result<Plan, AbiPlanError> {
    match (args, result) {
        ([CType::Int], CType::Int) => {
            let mut args = [Register::None; 6];
            args[0] = Register::Rdi;

            Ok(Plan {
                args,
                result: Register::Rax,
            })
        }
        _ => Err(AbiPlanError::NotSupport),
    }
}

/// # Safety
/// 非空参数数组必须包含 len 个有效的 CType；
/// plan 必须指向可写的 Plan。
/// args 为Null则意味着fn(void)
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
