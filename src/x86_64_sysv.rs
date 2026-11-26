use anyhow::*;
use core::{ffi::c_void, slice};

use crate::datastruct::{
    Allocation, Backend, CFunc, CType, CTypeKind, CallPlan, Layout, Place, Register, align_up,
};

// 指针:使用malloc
/// cbindgen:ignore
unsafe extern "C" {
    unsafe fn malloc(size: usize) -> *mut c_void;
    unsafe fn free(ptr: *mut c_void);
}

/// x86-64 SysV ABI 后端。
pub(crate) struct X86_64SysvBackend {
    //x64
}

pub(crate) struct X64SysvState {
    reg: usize,
    xmm_reg: usize,
    stack_offset: usize,
}
const X86_64_SYSV_INT_REG_LEN: usize = 6;
const X86_64_SYSV_XMM_REG_LEN: usize = 8;
const X86_64_SYSV_INT_REG: [Register; X86_64_SYSV_INT_REG_LEN] = [
    Register::Rdi,
    Register::Rsi,
    Register::Rdx,
    Register::Rcx,
    Register::R8,
    Register::R9,
];
const X86_64_SYSV_XMM_REG: [Register; X86_64_SYSV_XMM_REG_LEN] = [
    Register::Xmm0,
    Register::Xmm1,
    Register::Xmm2,
    Register::Xmm3,
    Register::Xmm4,
    Register::Xmm5,
    Register::Xmm6,
    Register::Xmm7,
];
impl X64SysvState {
    pub fn new() -> Self {
        // 0 初始化
        Self {
            reg: 0,
            xmm_reg: 0,
            stack_offset: 0,
        }
    }
    pub fn alloc(&mut self, layout: &Layout) -> Result<Place> {
        match layout.kind {
            CTypeKind::Int => match layout.size {
                0..=8 => {
                    if let Some(&reg) = X86_64_SYSV_INT_REG.get(self.reg) {
                        self.reg += 1;
                        return Ok(Place {
                            size: layout.size,
                            align: layout.align,
                            alloc: Allocation::Reg {
                                regs: [reg, Register::default()],
                                len: 1,
                            },
                        });
                    } else {
                        Ok(self.stack_alloc(layout))
                    }
                }
                16 => {if let Some(&[r0, r1]) = X86_64_SYSV_INT_REG.get(self.reg..self.reg+2) {
                        self.reg += 2;
                        return Ok(Place {
                            size: layout.size,
                            align: layout.align,
                            alloc: Allocation::Reg {
                                regs: [r0,r1],
                                len: 2,
                            },
                        });
                    } else {
                        Ok(self.stack_alloc(layout))
                    }
                }
                _ => bail!("Invalid int size {}", layout.size),
            },
            CTypeKind::Float => match layout.size {
                0..=8 => {
                    if let Some(&reg) = X86_64_SYSV_XMM_REG.get(self.xmm_reg) {
                        self.xmm_reg += 1;
                        Ok(Place {
                            size: layout.size,
                            align: layout.align,
                            alloc: Allocation::Reg {
                                regs: [reg, Register::default()],
                                len: 1,
                            },
                        })
                    } else {
                        Ok(self.stack_alloc(layout))
                    }
                }
                _ => bail!("Unsupported float size {}", layout.size),
            },
            kind => bail!("Unsupported arg kind {:?}", kind),
        }
    }
    fn stack_alloc(&mut self, layout: &Layout) -> Place {
        let align = layout.align.max(8);
        let offset = align_up(self.stack_offset, align);
        self.stack_offset = offset + align_up(layout.size, 8);
        Place {
            size: layout.size,
            align: layout.align,
            alloc: Allocation::Stack { offset },
        }
    }
}
pub(crate) struct X64SysPlanner<'a> {
    args: &'a [Layout],
    ret: Option<Layout>,
}

impl<'a> X64SysPlanner<'a> {
    pub fn new(args: &'a [Layout], ret: Option<Layout>) -> Self {
        X64SysPlanner { args, ret }
    }
    pub fn ret(&self) -> Result<Place> {
        let Some(ret) = &self.ret else {
            return Ok(Place {
                size: 0,
                align: 0,
                alloc: Allocation::Void,
            });
        };
        match ret.kind {
            CTypeKind::Int => match ret.size {
                0..=8 => Ok(Place {
                    size: ret.size,
                    align: ret.align,
                    alloc: Allocation::Reg {
                        regs: [Register::Rax, Register::default()],
                        len: 1,
                    },
                }),
                16 => Ok(Place {
                    size: ret.size,
                    align: ret.align,
                    alloc: Allocation::Reg {
                        regs: [Register::Rax, Register::Rdx],
                        len: 2,
                    },
                }),
                _ => {
                    bail!("Invalid int size:{}", ret.size)
                }
            },
            CTypeKind::Float => {
                if ret.size > 16 {
                    bail!("Invalid float size:{}", ret.size)
                }
                Ok(Place {
                    size: ret.size,
                    align: ret.align,
                    alloc: Allocation::Reg {
                        regs: [Register::Xmm0, Register::default()],
                        len: 1,
                    },
                })
            }
            t => {
                todo!("todo:impl return type{:?}", t)
            }
        }
    }
    pub fn plan(&self, mut on_arg: impl FnMut(Place)) -> Result<usize> {
        let mut state = X64SysvState::new();
        for arg in self.args {
            let p = state.alloc(arg)?;
            on_arg(p);
        }
        //SysV 要求 call 前 %rsp 16 字节对齐
        Ok(align_up(state.stack_offset, 16))
    }
}
impl Backend for X86_64SysvBackend {
    fn gen_plan(f: CFunc) -> Result<CallPlan> {
        let ret = match f.ret {
            CType::Void => None,
            other => Some(Layout::from(other)),
        };
        let args: Vec<Layout> = if f.args_len == 0 {
            &[]
        } else {
            unsafe { slice::from_raw_parts(f.args, f.args_len as usize) }
        }
        .iter()
        .map(|i| Layout::from(*i))
        .collect();
        let p = X64SysPlanner::new(&args, ret);
        let ret = p.ret()?;
        let args = unsafe { malloc(size_of::<Place>() * args.len()) } as *mut Place;
        let mut cur = 0usize;
        let ss = p.plan(|piece| {
            unsafe { *args.add(cur) = piece };
            cur += 1;
        })?;
        Ok(CallPlan {
            ret: ret,
            args: args,
            args_len: cur,
            stack_size: ss,
        })
    }
}
