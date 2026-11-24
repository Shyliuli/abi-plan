//! 数据层：`CFunc` 是输入（参数与返回类型），`CallPlan` 是输出；
//! 各后端通过 `Backend` 提供整数/浮点寄存器表。

use core::ffi::c_void;

// CallPlan 内的数组统一用 C 的 malloc/free 管理，C 调用方也可以直接 free。
unsafe extern "C" {
    unsafe fn malloc(size: usize) -> *mut c_void;
    unsafe fn free(ptr: *mut c_void);
}

/// 分配 `len` 个 `T`，空数组返回 null。
fn malloc_array<T>(len: usize) -> *mut T {
    if len == 0 {
        return core::ptr::null_mut();
    }
    // SAFETY: malloc 接受任意大小，返回的内存对任意基础类型对齐。
    let array = unsafe { malloc(len * size_of::<T>()) } as *mut T;
    assert!(!array.is_null(), "out of memory");
    array
}

unsafe fn free_regs(alloc: &Allocation) {
    if let Allocation::Reg { reg, .. } = alloc {
        // SAFETY: Reg 数组由 malloc 分配。
        unsafe { free(reg.cast()) };
    }
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
#[repr(C)]
/// C 类型。
///
/// 指针类型用 `Int { len: 8, .. }` 表示；`Void` 用于返回值声明，
/// 空参数函数可以是 `len == 1 && *args == Void` 或 `len == 0`。
pub enum CType {
    Int { len: usize, is_signed: bool },
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
/// `args` 与各 Reg 数组由 malloc 分配，用完后通过 `free_plan` 释放。
pub struct CallPlan {
    pub ret: Place,
    pub args: *mut Place,
    pub args_len: usize,
    pub stack_size: usize,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
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

#[derive(Debug, Clone, Copy)]
#[repr(C)]
/// 分配方式：寄存器、栈或 sret。
pub enum Allocation {
    Void,
    Reg { reg: *mut Register, len: usize },
    Stack { offset: usize },
    Sret,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
/// 值的大小、对齐与存放位置。
pub struct Place {
    pub size: usize,
    pub align: usize,
    pub alloc: Allocation,
}

// 聚合体分类与布局字段为后续 eightbyte 分类预留，当前标量路径暂不使用。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CTypeKind {
    Int,
    Float,
    Void,
    Struct,
    Array,
    Union,
}

#[allow(dead_code)]
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
    /// 只计算标量布局；聚合体与非法宽度返回 `None`。
    pub(crate) fn from_ctype(ctype: &CType) -> Option<Self> {
        let (size, align, kind) = match *ctype {
            CType::Void => (0, 1, CTypeKind::Void),
            CType::Int { len, .. } if matches!(len, 1 | 2 | 4 | 8 | 16) => (len, len, CTypeKind::Int),
            CType::Float { len } if matches!(len, 4 | 8 | 16) => (len, len, CTypeKind::Float),
            _ => return None,
        };
        Some(Self { size, align, offset: Vec::new(), sub_layout: Vec::new(), kind })
    }
}

impl Place {
    pub(crate) fn void() -> Self {
        Self { size: 0, align: 1, alloc: Allocation::Void }
    }

    pub(crate) fn reg(size: usize, align: usize, registers: &[Register]) -> Self {
        let reg = malloc_array(registers.len());
        // memcpy
        unsafe { core::ptr::copy_nonoverlapping(registers.as_ptr(), reg, registers.len()) };
        Self { size, align, alloc: Allocation::Reg { reg, len: registers.len() } }
    }

    pub(crate) fn stack(size: usize, align: usize, offset: usize) -> Self {
        Self { size, align, alloc: Allocation::Stack { offset } }
    }

    /// 返回该位置分配的寄存器切片；非寄存器分配返回 `None`。
    pub fn registers(&self) -> Option<&[Register]> {
        let Allocation::Reg { reg, len } = self.alloc else {
            return None;
        };
        // SAFETY: `reg` 指向 malloc 分配的 `len` 个 `Register`。
        Some(unsafe { core::slice::from_raw_parts(reg, len) })
    }
}

impl CallPlan {
    /// 把 `args` 拷贝进 malloc 内存并接管所有权。
    pub(crate) fn new(ret: Place, args: &[Place], stack_size: usize) -> Self {
        let array = malloc_array(args.len());
        // SAFETY: `array` 指向足够容纳 `args` 的未初始化内存，两者不重叠。
        unsafe { core::ptr::copy_nonoverlapping(args.as_ptr(), array, args.len()) };
        Self { ret, args: array, args_len: args.len(), stack_size }
    }

    /// 返回由 `get_plan` 构造的参数位置切片。
    pub fn args(&self) -> &[Place] {
        if self.args.is_null() {
            return &[];
        }
        // SAFETY: `args` 指向 malloc 分配的 `args_len` 个 `Place`。
        unsafe { core::slice::from_raw_parts(self.args, self.args_len) }
    }

    /// 回收 `args` 与所有寄存器数组；调用后不得再访问。
    pub(crate) unsafe fn free(self) {
        for place in self.args() {
            // SAFETY: 各 Place 的 Reg 数组由 malloc 分配。
            unsafe { free_regs(&place.alloc) };
        }
        // SAFETY: 返回值的 Reg 数组由 malloc 分配。
        unsafe { free_regs(&self.ret.alloc) };
        // SAFETY: `args` 由 malloc 分配；空数组为 null，`free(null)` 无操作。
        unsafe { free(self.args.cast()) };
    }
}

/// 目标后端：只提供寄存器表，分配逻辑由共享的 `Allocator` 完成。
pub(crate) trait Backend {
    const INT_REGS: &'static [Register];
    const FP_REGS: &'static [Register];

    fn gen_plan(args: &[Layout], ret: Layout) -> CallPlan;
}
