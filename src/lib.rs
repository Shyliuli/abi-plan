mod datastruct;

pub use datastruct::{Allocation, CFunc, CallPlan, CType, Place, Register};
use datastruct::X86_64SysvBackend;

use crate::datastruct::Backend;
#[unsafe(no_mangle)]
pub extern "C" fn abi_plan_gen(f: CFunc) -> CallPlan {
    X86_64SysvBackend::gen_plan(f)
}