use crate::allocator::Allocator;
use crate::datastruct::{Backend, CallPlan, Layout, Register};

/// x86_64 System V 后端只提供寄存器表，参数/返回值分配逻辑由共享的 `Allocator` 完成。
/// 其他后端实现同一 `Backend` trait 即可复用分配器。
pub(crate) struct X86_64SysBackend;

impl Backend for X86_64SysBackend {
    const INT_REGS: &'static [Register] = &[
        Register::Rdi,
        Register::Rsi,
        Register::Rdx,
        Register::Rcx,
        Register::R8,
        Register::R9,
    ];
    const FP_REGS: &'static [Register] = &[
        Register::Xmm0,
        Register::Xmm1,
        Register::Xmm2,
        Register::Xmm3,
        Register::Xmm4,
        Register::Xmm5,
        Register::Xmm6,
        Register::Xmm7,
    ];

    fn gen_plan(args: &[Layout], ret: Layout) -> CallPlan {
        Allocator::new(Self::INT_REGS, Self::FP_REGS).plan(args, ret)
    }
}
