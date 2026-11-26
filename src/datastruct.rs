use anyhow::*;
use core::{ffi::c_void, slice};
use std::mem;
// 指针:使用malloc
/// cbindgen:ignore
unsafe extern "C" {
    unsafe fn malloc(size: usize) -> *mut c_void;
    unsafe fn free(ptr: *mut c_void);
}
#[inline]
fn align_up(val: usize, align: usize) -> usize {
    assert!(align.is_power_of_two());
    (val + align - 1) & !(align - 1)
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
/// CFunc C 函数声明。
///
/// - `args` 参数列表，`args_len` 为其长度，`ret` 为返回值类型。
/// 由外部程序构造后作为请求发送。
pub struct CFunc {
    pub args: *mut CType,
    pub args_len: u32,
    pub ret: CType,
}

#[derive(Debug, Clone, Copy)]
#[repr(C, u32)]
/// C 类型。
///
/// 指针类型用 `Int { len: 8, .. }` 表示；`Void` 用于返回值声明，
/// 空参数函数可以是 `len == 1 && *args == Void` 或 `len == 0`。
pub enum CType {
    Int { len: usize },
    Float { len: usize },
    Void,
    // 为 C 兼容布局使用 ptr + len。
    Struct { ptr: *mut CType, len: usize },
    Array { item: *mut CType, len: usize },
    Union { ptr: *mut CType, len: usize },
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
/// 调用计划，作为 abi_plan 库的计算结果，指导外部程序进行 ABI lowering。
///
/// `args` 由 malloc 分配，用完后通过 `free_plan` 释放。
pub struct CallPlan {
    pub ret: Place,
    pub args: *mut Place,
    pub args_len: usize,
    pub stack_size: usize,
}

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
/// 寄存器，后续可扩展为按 ABI 返回不同的寄存器。
pub enum Register {
    Rax,
    Rcx,
    Rdx,
    Rbx,
    Rsp,
    Rbp,
    Rsi,
    Rdi,
    R8,
    R9,
    Xmm0,
    Xmm1,
    Xmm2,
    Xmm3,
    Xmm4,
    Xmm5,
    Xmm6,
    Xmm7,
}
impl Default for Register {
    fn default() -> Self {
        //如果你不关心它是什么
        Register::Rax
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
/// 分配方式：寄存器、栈或 sret。
pub enum Allocation {
    Void,
    Reg { regs: [Register; 2], len: u8 },
    Stack { offset: usize },
    Sret,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Place {
    pub size: usize,
    pub align: usize,
    pub alloc: Allocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CTypeKind {
    Int,
    Float,
    Struct,
    Array,
    Union,
}

#[derive(Debug, Clone)]
pub(crate) struct Layout {
    pub(crate) size: usize,
    pub(crate) align: usize,
    // 主要用于 union 和 struct 的字段偏移量。
    pub(crate) offset: Vec<usize>,
    // 子类型。
    pub(crate) sub_layout: Vec<Layout>,
    pub(crate) kind: CTypeKind,
}
impl Layout {
    pub fn new(kind: CTypeKind) -> Self {
        Layout {
            size: 0,
            align: 0,
            offset: Vec::new(),
            sub_layout: Vec::new(),
            kind,
        }
    }
}
impl From<CType> for Layout {
    fn from(t: CType) -> Self {
        match t {
            CType::Int { len } => {
                // 标量align往往是自然对齐...
                Layout {
                    size: len,
                    align: len,
                    // offset: 用于聚合类型
                    offset: vec![],
                    // sub_layout: 用于聚合类型
                    sub_layout: vec![],
                    kind: CTypeKind::Int,
                }
            }
            CType::Float { len } => Layout {
                size: len,
                align: len,
                offset: vec![],
                sub_layout: vec![],
                kind: CTypeKind::Float,
            },
            CType::Void => {
                panic!("void cannot be a field/element");
            }
            CType::Struct { ptr, len } => {
                let mut current_offset = 0;
                let mut max_align = 1;
                let mut offset: Vec<usize> = Vec::with_capacity(len);
                let mut sub_layout: Vec<Layout> = Vec::with_capacity(len);

                let s = if len == 0 {
                    &[]
                } else {
                    unsafe { slice::from_raw_parts(ptr, len) }
                };
                for i in s {
                    let layout = Layout::from(*i);
                    // 字段起点
                    current_offset = align_up(current_offset, layout.align);
                    offset.push(current_offset);
                    current_offset += layout.size;
                    max_align = max_align.max(layout.align);
                    sub_layout.push(layout);
                }
                Layout {
                    size: align_up(current_offset, max_align),
                    align: max_align,
                    offset,
                    sub_layout,
                    kind: CTypeKind::Struct,
                }
            }
            CType::Array { item, len } => {
                let layout = Layout::from(unsafe { *item });
                Layout {
                    size: layout.size * len,
                    align: layout.align,
                    offset: vec![],
                    sub_layout: vec![layout],
                    kind: CTypeKind::Array,
                }
            }
            CType::Union { ptr, len } => {
                let mut max_size = 0;
                let mut max_align = 1;
                let mut sub_layout: Vec<Layout> = Vec::with_capacity(len);
                let s = unsafe { slice::from_raw_parts(ptr, len) };
                for i in s {
                    let layout = Layout::from(*i);
                    max_size = max_size.max(layout.size);
                    max_align = max_align.max(layout.align);
                    sub_layout.push(layout);
                }
                Layout {
                    size: align_up(max_size, max_align),
                    align: max_align,
                    offset: vec![],
                    sub_layout,
                    kind: CTypeKind::Union,
                }
            }
        }
    }
}

pub(crate) struct X86_64SysvBackend {
    //x64
}
pub trait Backend {
    //输入CFunc返回Plan
    fn gen_plan(f: CFunc) -> Result<CallPlan>;
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
