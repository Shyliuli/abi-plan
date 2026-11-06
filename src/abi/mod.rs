//! ABI classifiers that turn a C signature into a byte-movement plan.
use crate::types::*;

mod common;
mod shy;

pub fn plan(abi: AbiKind, signature: &FunctionSignature) -> Result<CallPlan, AbiError> {
    match abi {
        AbiKind::ShyV1 => shy::plan(signature),
        AbiKind::SysV64 => common::sysv64(signature),
        AbiKind::WindowsX64 => common::windows_x64(signature),
        AbiKind::Aapcs64 => common::aapcs64(signature),
        AbiKind::Riscv64 => common::riscv64(signature),
        AbiKind::Riscv32 => common::riscv32(signature),
        AbiKind::LoongArch64 => common::loongarch64(signature),
    }
}

pub(crate) fn base(abi: AbiKind, sig: &FunctionSignature) -> CallPlan {
    CallPlan {
        abi,
        signature: sig.clone(),
        args: Vec::with_capacity(sig.params.len()),
        return_value: ReturnPlan::Void,
        gpr: Vec::new(),
        fpr: Vec::new(),
        vector: Vec::new(),
        stack: StackPlan {
            argument_bytes: 0,
            alignment: abi.data_layout().stack_align,
            shadow_bytes: 0,
            return_address_bytes: 0,
        },
    }
}

pub(crate) fn add_register(
    plan: &mut CallPlan,
    register: RegisterId,
    source: Option<(usize, u32, u32)>,
) {
    let item = RegisterUse { register, source };
    match register.class {
        RegisterClass::Gpr => plan.gpr.push(item),
        RegisterClass::Fpr => plan.fpr.push(item),
        RegisterClass::Vector => plan.vector.push(item),
    }
}

pub(crate) fn add_stack(plan: &mut CallPlan, size: u32, align: u16, negative: bool) -> Location {
    let aligned = align_up(plan.stack.argument_bytes, align.max(1) as u32);
    let end = aligned + size;
    plan.stack.argument_bytes = end;
    Location::Stack {
        offset: if negative {
            -(end as i32)
        } else {
            aligned as i32
        },
    }
}

pub(crate) fn scalar_parts(ty: &AbiType, location: Location, source_offset: u32) -> Vec<ValuePart> {
    vec![ValuePart {
        source_offset,
        size: ty.size(),
        location,
    }]
}

pub(crate) fn align_up(value: u32, align: u32) -> u32 {
    if align <= 1 {
        value
    } else {
        value.div_ceil(align) * align
    }
}

pub(crate) fn direct_return(plan: &mut CallPlan, ty: &AbiType, class: RegisterClass, index: u16) {
    if ty.is_void() {
        plan.return_value = ReturnPlan::Void;
        return;
    }
    let part = ValuePart {
        source_offset: 0,
        size: ty.size(),
        location: Location::Register(RegisterId { class, index }),
    };
    plan.return_value = ReturnPlan::Direct { parts: vec![part] };
}

pub(crate) fn indirect_return(plan: &mut CallPlan, ty: &AbiType, index: u16) {
    let ptr = Location::Register(RegisterId {
        class: RegisterClass::Gpr,
        index,
    });
    plan.return_value = ReturnPlan::Indirect {
        pointer: ptr,
        size: ty.size(),
        align: ty.align(),
    };
    add_register(plan, ptr_register(index), None);
}

fn ptr_register(index: u16) -> RegisterId {
    RegisterId {
        class: RegisterClass::Gpr,
        index,
    }
}

pub(crate) struct RegisterBudget {
    pub gpr: u16,
    pub fpr: u16,
    pub max_gpr: u16,
    pub max_fpr: u16,
    pub indirect_threshold: u32,
    /// SysV copies large aggregates into the callee's frame (`byval`), while
    /// AArch64, RISC-V, and LoongArch pass a plain reference to the caller's
    /// temporary. The flag records which convention the target uses.
    pub byval_aggregates: bool,
}

pub(crate) fn scalar_or_indirect(
    plan: &mut CallPlan,
    index: usize,
    ty: &AbiType,
    budget: &mut RegisterBudget,
) -> Result<(), AbiError> {
    let size = ty.size();
    if ty.is_aggregate() && size > budget.indirect_threshold {
        if budget.gpr >= budget.max_gpr {
            let location = add_stack(plan, 8, 8, false);
            plan.args.push(ArgPlan {
                index,
                ty: ty.clone(),
                passing: Passing::Indirect {
                    pointer: location,
                    size,
                    align: ty.align(),
                    byval: budget.byval_aggregates,
                },
            });
            return Ok(());
        }
        let loc = Location::Register(RegisterId {
            class: RegisterClass::Gpr,
            index: budget.gpr,
        });
        budget.gpr += 1;
        add_register(
            plan,
            RegisterId {
                class: RegisterClass::Gpr,
                index: budget.gpr - 1,
            },
            Some((index, 0, 8)),
        );
        plan.args.push(ArgPlan {
            index,
            ty: ty.clone(),
            passing: Passing::Indirect {
                pointer: loc,
                size,
                align: ty.align(),
                byval: budget.byval_aggregates,
            },
        });
        return Ok(());
    }
    if ty.is_float() {
        if budget.fpr >= budget.max_fpr {
            // Out of registers: the argument continues on the stack.
            let location = add_stack(plan, 8, 8, false);
            plan.args.push(ArgPlan {
                index,
                ty: ty.clone(),
                passing: Passing::Direct {
                    parts: vec![ValuePart {
                        source_offset: 0,
                        size,
                        location,
                    }],
                },
            });
            return Ok(());
        }
        let loc = Location::Register(RegisterId {
            class: RegisterClass::Fpr,
            index: budget.fpr,
        });
        add_register(
            plan,
            RegisterId {
                class: RegisterClass::Fpr,
                index: budget.fpr,
            },
            Some((index, 0, size)),
        );
        budget.fpr += 1;
        plan.args.push(ArgPlan {
            index,
            ty: ty.clone(),
            passing: Passing::Direct {
                parts: scalar_parts(ty, loc, 0),
            },
        });
        return Ok(());
    }
    if budget.gpr >= budget.max_gpr {
        let location = add_stack(plan, size.max(8), 8, false);
        plan.args.push(ArgPlan {
            index,
            ty: ty.clone(),
            passing: Passing::Direct {
                parts: vec![ValuePart {
                    source_offset: 0,
                    size,
                    location,
                }],
            },
        });
        return Ok(());
    }
    let loc = Location::Register(RegisterId {
        class: RegisterClass::Gpr,
        index: budget.gpr,
    });
    add_register(
        plan,
        RegisterId {
            class: RegisterClass::Gpr,
            index: budget.gpr,
        },
        Some((index, 0, size.min(8))),
    );
    budget.gpr += 1;
    plan.args.push(ArgPlan {
        index,
        ty: ty.clone(),
        passing: Passing::Direct {
            parts: scalar_parts(ty, loc, 0),
        },
    });
    Ok(())
}
