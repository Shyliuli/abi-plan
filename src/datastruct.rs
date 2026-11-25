use core::{ffi::c_void, slice};

// 指针:使用malloc
unsafe extern "C" {
    unsafe  fn malloc(size: usize) -> *mut c_void;
    unsafe  fn free(ptr: *mut c_void);
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
pub struct Place {
    pub size: usize,
    pub align: usize,
    pub alloc: Allocation,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CTypeKind {
    Int,
    Float,
    Void,
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
impl Layout{
    pub fn new(kind:CTypeKind)->Self{
        Layout{
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
           match t{
               CType::Int{len,is_signed} =>{
                   // 标量align往往是自然对齐...
                   Layout{
                       size: len,
                       align: len,
                       // offset: 用于聚合类型
                       offset: vec![],
                       // sub_layout: 用于聚合类型
                       sub_layout: vec![],
                       kind: CTypeKind::Int,
                   }
               },
               CType::Float{len} =>{
                    Layout{
                        size: len,
                        align: len,
                        offset: vec![],
                        sub_layout: vec![],
                        kind: CTypeKind::Float,
                    }
                },
                CType::Void =>{
                    panic!("void cannot be a field/element");
                },
                CType::Struct{ptr,len} =>{
                    let mut current_offset=0;
                    let mut max_align = 1;
                    let mut offset:Vec<usize>=Vec::with_capacity(len);
                    let mut sub_layout:Vec<Layout> = Vec::with_capacity(len);

                    let s = if len == 0 {
                        &[]
                    }else{
                    unsafe{slice::from_raw_parts(ptr, len)}
                    };
                    for i in s{
                        let layout=Layout::from(*i);
                        // 字段起点
                        current_offset=align_up(current_offset,layout.align);
                        offset.push(current_offset);
                        current_offset+=layout.size;
                        max_align=max_align.max(layout.align);
                        sub_layout.push(layout);
                    }
                    Layout { size: align_up(current_offset, max_align)
                    ,align: max_align, offset, sub_layout, kind: CTypeKind::Struct}
                },
                CType::Array{item,len} =>{
                    let layout=Layout::from(unsafe{*item});
                    Layout { size: layout.size*len,
                        align: layout.align
                        , offset: vec![]
                        , sub_layout: vec![layout]
                        , kind: CTypeKind::Array}
                },
                CType::Union{ptr,len} =>{
                    let mut max_size = 0;
                    let mut max_align = 1;
                    let mut sub_layout:Vec<Layout> = Vec::with_capacity(len);
                    let s = unsafe{slice::from_raw_parts(ptr, len)};
                    for i in s{
                        let layout=Layout::from(*i);
                        max_size=max_size.max(layout.size);
                        max_align=max_align.max(layout.align);
                        sub_layout.push(layout);
                    }
                    Layout { size: align_up(max_size,max_align)
                    ,align: max_align, offset: vec![0], sub_layout, kind: CTypeKind::Union}
                }
            }
    }
}

pub(crate) struct X86_64SysvBackend {
    //x64
}
pub trait Backend {
    //输入CFunc返回Plan
    fn gen_plan(f:CFunc)->CallPlan;
}

impl Backend for X86_64SysvBackend{
    fn gen_plan(f:CFunc)->CallPlan{
        // 1. ret
        let ret = Layout::from(f.ret);
        todo!("impl gen_plan for x86_64 backend");
    }
}