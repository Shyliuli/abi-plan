use core::cell::RefCell;
use core::ffi::c_char;
use std::io::Write;

mod datastruct;
mod x86_64_sysv;

pub use datastruct::{Allocation, CFunc, CallPlan, CType, Place, Register};
use x86_64_sysv::X86_64SysvBackend;

use crate::datastruct::Backend;

thread_local! {
    static ERR_BUF: RefCell<[u8; 256]> = const { RefCell::new([0; 256]) };
}

fn set_error(args: core::fmt::Arguments<'_>) {
    ERR_BUF.with(|buf| {
        let mut b = buf.borrow_mut();
        b.fill(0);
        let mut cur = &mut b[..255];
        let _ = cur.write_fmt(args);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn abi_plan_error() -> *const c_char {
    ERR_BUF.with(|buf| buf.borrow().as_ptr() as *const c_char)
}

#[unsafe(no_mangle)]
pub extern "C" fn abi_plan_gen(f: CFunc, p: *mut CallPlan) -> i32 {
    if p.is_null() {
        set_error(format_args!("abi_plan_gen got a nullptr"));
        return -1;
    }
    match X86_64SysvBackend::gen_plan(f) {
        Ok(plan) => {
            unsafe { *p = plan };
            0
        }
        Err(e) => {
            set_error(format_args!("{e}"));
            -1
        }
    }
}