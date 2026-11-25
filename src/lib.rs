//! C ABI 边界：把 C 函数签名（`CFunc` 的 arg/ret）转换为 `CallPlan`。
//!
//! - `datastruct`：数据层，定义请求/结果类型与后端 trait。
//! - `x86_64_sys`：x86_64 SysV 后端，只提供寄存器表。
//! - `allocator`：共享分配器，按参数顺序生成 `Place`。

use std::slice;

mod allocator;
mod datastruct;
mod x86_64_sys;

use datastruct::Backend;

pub use datastruct::{Allocation, CFunc, CallPlan, CType, Place, Register};

/// `get_plan` 的错误码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum AbiPlanError {
    Ok = 0,
    Unsupported = 1,
    InvalidArgument = 2,
}

/// # Safety
/// `cfunc.args` 在 `args_len > 0` 时必须指向 `args_len` 个已初始化的 `CType`；
/// `out` 必须指向可写的 `CallPlan`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_plan(cfunc: CFunc, out: *mut CallPlan) -> AbiPlanError {
    if out.is_null() || (cfunc.args_len > 0 && cfunc.args.is_null()) {
        return AbiPlanError::InvalidArgument;
    }

    // args
    let args = if cfunc.args_len == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(cfunc.args, cfunc.args_len as usize) }
    };

    // `(void)` 形式的无参声明等同于 `()`；其余情况下出现 Void 参数会让
    // `plan.args()[i]` 与调用方的 `val[i]` 错位，直接拒绝。
    let params: &[CType] = if args.len() == 1 && matches!(args[0], CType::Void) {
        &[]
    } else {
        args
    };

    // Todo: 支持结构体
    // 在这里初始化
    let mut layouts = Vec::with_capacity(params.len());

    for ctype in params {
        if matches!(ctype, CType::Void) {
            return AbiPlanError::InvalidArgument;
        }
        // CType->Layout
        match datastruct::Layout::from_ctype(ctype) {
            Some(layout) => layouts.push(layout),
            None => return AbiPlanError::Unsupported,
        }
    }
    
    // 返回值
    let Some(ret) = datastruct::Layout::from_ctype(&cfunc.ret) else {
        return AbiPlanError::Unsupported;
    };

    // 生成plan
    let plan = x86_64_sys::X86_64SysBackend::gen_plan(&layouts, ret);
    
    // SAFETY: `out` 已检查非空。
    unsafe { out.write(plan) };
    AbiPlanError::Ok
}

/// # Safety
/// `plan` 必须由 `get_plan` 返回且只释放一次；释放后不得再访问。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_plan(plan: CallPlan) {
    // SAFETY: 见函数文档。
    unsafe { plan.free() };
}
