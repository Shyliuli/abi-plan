//! 共享分配器：按参数顺序消费整数/浮点游标，为每个参数生成 `Place`；
//! 游标耗尽时转栈，返回值按整数/浮点分别分类。

use crate::datastruct::{CTypeKind, CallPlan, Layout, Place, Register};

/// 栈参数按 8 字节槽排布。
const SLOT: usize = 8;
/// 调用时 `%rsp` 需 16 字节对齐，栈大小向上取整。
const STACK_ALIGN: usize = 16;

fn align_up(value: usize, align: usize) -> usize {
    value.div_ceil(align) * align
}

fn slot_count(size: usize) -> usize {
    size.div_ceil(SLOT)
}

/// 寄存器表与当前位置。
#[derive(Debug, Clone, Copy)]
pub(crate) struct RegCursor {
    regs: &'static [Register],
    pos: usize,
}

impl RegCursor {
    fn new(regs: &'static [Register]) -> Self {
        Self { regs, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.regs.len() - self.pos
    }

    /// 取出接下来的 `count` 个寄存器，调用方保证足够。
    fn take(&mut self, count: usize) -> &'static [Register] {
        let regs = &self.regs[self.pos..self.pos + count];
        self.pos += count;
        regs
    }
}

/// 迭代器，按参数顺序分配参数。
pub(crate) struct Allocator {
    int: RegCursor,
    fp: RegCursor,
    stack_offset: usize,
}

impl Allocator {
    pub(crate) fn new(int_regs: &'static [Register], fp_regs: &'static [Register]) -> Self {
        Self {
            int: RegCursor::new(int_regs),
            fp: RegCursor::new(fp_regs),
            stack_offset: 0,
        }
    }

    fn cursor(&mut self, is_int: bool) -> &mut RegCursor {
        if is_int {
            &mut self.int
        } else {
            &mut self.fp
        }
    }

    /// 返回值分类
    pub(crate) fn reserve_return(&mut self, ret: &Layout) -> Place {
        match ret.kind {
            CTypeKind::Void => Place::void(),
            // 不超过 8 字节用 Rax，16 字节用 Rdx:Rax。
            CTypeKind::Int if ret.size == 16 => Place::reg(ret.size, ret.align, &[Register::Rax, Register::Rdx]),
            CTypeKind::Int => Place::reg(ret.size, ret.align, &[Register::Rax]),
            // 不超过 8 字节用 Xmm0，16 字节用 Xmm1:Xmm0。
            CTypeKind::Float if ret.size == 16 => Place::reg(ret.size, ret.align, &[Register::Xmm0, Register::Xmm1]),
            CTypeKind::Float => Place::reg(ret.size, ret.align, &[Register::Xmm0]),
            // `Layout::from_ctype` 只产生标量布局。
            CTypeKind::Struct | CTypeKind::Array | CTypeKind::Union => unreachable!(),
        }
    }

    /// 迭代器:根据参数和目前寄存器/栈状态给定{lace
    pub(crate) fn next(&mut self, layout: &Layout) -> Place {
        let is_int = layout.kind == CTypeKind::Int;
        // 计算需要的slot数(size/8)
        let count = slot_count(layout.size);
        // 如果寄存器不够，则转栈
        if self.cursor(is_int).remaining() < count {
            return self.spill(layout);
        }
        // 寄存器路径
        Place::reg(layout.size, layout.align, self.cursor(is_int).take(count))
    }

    /// 分配栈位置
    fn spill(&mut self, layout: &Layout) -> Place {
        let offset = align_up(self.stack_offset, layout.align.max(SLOT));
        self.stack_offset = offset + slot_count(layout.size) * SLOT;
        Place::stack(layout.size, layout.align, offset)
    }

    /// plan生成
    pub(crate) fn plan(mut self, args: &[Layout], ret: Layout) -> CallPlan {
        let ret_place = self.reserve_return(&ret);
        // 每一个参数生成一个Place，核心算法见self.next()
        let places: Vec<Place> = args.iter().map(|arg| self.next(arg)).collect();
        // 栈大小
        let stack_size = align_up(self.stack_offset, STACK_ALIGN);
        CallPlan::new(ret_place, &places, stack_size)
    }
}
