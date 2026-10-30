//! Common classifiers for mainstream 64-bit C ABIs.
//!
//! These routines deliberately keep the source object layout and transport
//! layout separate. They cover ordinary scalar calls and the common small
//! aggregate cases; unusual vector, long-double, packed, and platform-specific
//! extensions return a diagnostic instead of silently choosing a wrong plan.
use super::*;

pub fn sysv64(sig: &FunctionSignature) -> Result<CallPlan, AbiError> {
    aggregate_register_abi(AbiKind::SysV64, sig, 6, 8, 16, 8, false)
}

pub fn windows_x64(sig: &FunctionSignature) -> Result<CallPlan, AbiError> {
    let mut plan = base(AbiKind::WindowsX64, sig);
    plan.stack.shadow_bytes = 32;

    // Win64 assigns *positions*, not two independent register files: the first
    // four arguments take RCX/RDX/R8/R9 or XMM0..XMM3 by position, and the
    // fifth argument goes on the stack no matter how few registers were used.
    let mut position = 0u16;
    if sig.ret.is_aggregate() && sig.ret.size() > 8 {
        indirect_return(&mut plan, &sig.ret, 0);
        position = 1;
    } else if !sig.ret.is_void() {
        let class = if sig.ret.is_float() {
            RegisterClass::Fpr
        } else {
            RegisterClass::Gpr
        };
        direct_return(&mut plan, &sig.ret, class, 0);
    }

    for (index, ty) in sig.params.iter().enumerate() {
        let aggregate = ty.is_aggregate();
        // Aggregates of 1, 2, 4, or 8 bytes travel by value in a register;
        // every other aggregate is passed by reference.
        let by_value = !aggregate || matches!(ty.size(), 1 | 2 | 4 | 8);
        if position < 4 {
            let class = if ty.is_float() {
                RegisterClass::Fpr
            } else {
                RegisterClass::Gpr
            };
            let location = Location::Register(RegisterId {
                class,
                index: position,
            });
            add_register(
                &mut plan,
                RegisterId {
                    class,
                    index: position,
                },
                Some((index, 0, ty.size().min(8))),
            );
            if !by_value {
                plan.args.push(ArgPlan {
                    index,
                    ty: ty.clone(),
                    passing: Passing::Indirect {
                        pointer: location,
                        size: ty.size(),
                        align: ty.align(),
                        byval: false,
                    },
                });
            } else {
                plan.args.push(ArgPlan {
                    index,
                    ty: ty.clone(),
                    passing: Passing::Direct {
                        parts: scalar_parts(ty, location, 0),
                    },
                });
            }
        } else {
            // Stack slots are eight bytes for both a value and a pointer.
            let size = align_up(ty.size().max(8), 8);
            let location = add_stack(&mut plan, size, 8, false);
            if !by_value {
                plan.args.push(ArgPlan {
                    index,
                    ty: ty.clone(),
                    passing: Passing::Indirect {
                        pointer: location,
                        size: ty.size(),
                        align: ty.align(),
                        byval: false,
                    },
                });
            } else {
                plan.args.push(ArgPlan {
                    index,
                    ty: ty.clone(),
                    passing: Passing::Direct {
                        parts: vec![ValuePart {
                            source_offset: 0,
                            size: ty.size(),
                            location,
                        }],
                    },
                });
            }
        }
        position += 1;
    }
    Ok(plan)
}

pub fn aapcs64(sig: &FunctionSignature) -> Result<CallPlan, AbiError> {
    arm_like(AbiKind::Aapcs64, sig)
}

pub fn riscv64(sig: &FunctionSignature) -> Result<CallPlan, AbiError> {
    riscv_like(AbiKind::Riscv64, sig)
}

pub fn loongarch64(sig: &FunctionSignature) -> Result<CallPlan, AbiError> {
    riscv_like(AbiKind::LoongArch64, sig)
}

fn aggregate_register_abi(
    abi: AbiKind,
    sig: &FunctionSignature,
    gpr_limit: u16,
    fpr_limit: u16,
    aggregate_limit: u32,
    eightbyte: u32,
    _windows_style: bool,
) -> Result<CallPlan, AbiError> {
    let mut plan = base(abi, sig);
    let mut gpr = 0u16;
    let mut fpr = 0u16;
    if sig.ret.is_aggregate() {
        if sig.ret.size() > aggregate_limit {
            indirect_return(&mut plan, &sig.ret, 0);
            gpr = 1;
        } else {
            let classes = aggregate_classes(&sig.ret, eightbyte);
            let needed_g = classes.iter().filter(|c| **c == RegisterClass::Gpr).count() as u16;
            let needed_f = classes.iter().filter(|c| **c == RegisterClass::Fpr).count() as u16;
            if gpr + needed_g > gpr_limit || fpr + needed_f > fpr_limit {
                indirect_return(&mut plan, &sig.ret, 0);
                gpr = 1;
            } else {
                let mut parts = Vec::new();
                for (n, class) in classes.iter().enumerate() {
                    let index = if *class == RegisterClass::Gpr {
                        let i = gpr;
                        gpr += 1;
                        i
                    } else {
                        let i = fpr;
                        fpr += 1;
                        i
                    };
                    let size = (sig.ret.size() - n as u32 * eightbyte).min(eightbyte);
                    let loc = Location::Register(RegisterId {
                        class: *class,
                        index,
                    });
                    parts.push(ValuePart {
                        source_offset: n as u32 * eightbyte,
                        size,
                        location: loc,
                    });
                    add_register(
                        &mut plan,
                        RegisterId {
                            class: *class,
                            index,
                        },
                        None,
                    );
                }
                plan.return_value = ReturnPlan::Direct { parts };
            }
        }
    } else if !sig.ret.is_void() {
        let class = if sig.ret.is_float() {
            RegisterClass::Fpr
        } else {
            RegisterClass::Gpr
        };
        direct_return(&mut plan, &sig.ret, class, 0);
    }

    for (index, ty) in sig.params.iter().enumerate() {
        if ty.is_aggregate() {
            if ty.size() > aggregate_limit {
                let loc = if gpr < gpr_limit {
                    let loc = Location::Register(RegisterId {
                        class: RegisterClass::Gpr,
                        index: gpr,
                    });
                    add_register(
                        &mut plan,
                        RegisterId {
                            class: RegisterClass::Gpr,
                            index: gpr,
                        },
                        Some((index, 0, 8)),
                    );
                    gpr += 1;
                    loc
                } else {
                    add_stack(&mut plan, 8, 8, false)
                };
                plan.args.push(ArgPlan {
                    index,
                    ty: ty.clone(),
                    passing: Passing::Indirect {
                        pointer: loc,
                        size: ty.size(),
                        align: ty.align(),
                        byval: true,
                    },
                });
                continue;
            }
            let classes = aggregate_classes(ty, eightbyte);
            let needed_g = classes.iter().filter(|c| **c == RegisterClass::Gpr).count() as u16;
            let needed_f = classes.iter().filter(|c| **c == RegisterClass::Fpr).count() as u16;
            if gpr + needed_g <= gpr_limit && fpr + needed_f <= fpr_limit {
                let mut parts = Vec::new();
                for (n, class) in classes.iter().enumerate() {
                    let index_reg = if *class == RegisterClass::Gpr {
                        let i = gpr;
                        gpr += 1;
                        i
                    } else {
                        let i = fpr;
                        fpr += 1;
                        i
                    };
                    let size = (ty.size() - n as u32 * eightbyte).min(eightbyte);
                    let loc = Location::Register(RegisterId {
                        class: *class,
                        index: index_reg,
                    });
                    parts.push(ValuePart {
                        source_offset: n as u32 * eightbyte,
                        size,
                        location: loc,
                    });
                    add_register(
                        &mut plan,
                        RegisterId {
                            class: *class,
                            index: index_reg,
                        },
                        Some((index, n as u32 * eightbyte, size)),
                    );
                }
                plan.args.push(ArgPlan {
                    index,
                    ty: ty.clone(),
                    passing: Passing::Direct { parts },
                });
            } else {
                let loc = add_stack(&mut plan, align_up(ty.size(), eightbyte), 8, false);
                plan.args.push(ArgPlan {
                    index,
                    ty: ty.clone(),
                    passing: Passing::Direct {
                        parts: vec![ValuePart {
                            source_offset: 0,
                            size: ty.size(),
                            location: loc,
                        }],
                    },
                });
            }
            continue;
        }
        let mut budget = RegisterBudget {
            gpr,
            fpr,
            max_gpr: gpr_limit,
            max_fpr: fpr_limit,
            indirect_threshold: aggregate_limit,
            byval_aggregates: true,
        };
        scalar_or_indirect(&mut plan, index, ty, &mut budget)?;
        gpr = budget.gpr;
        fpr = budget.fpr;
    }
    Ok(plan)
}

fn arm_like(abi: AbiKind, sig: &FunctionSignature) -> Result<CallPlan, AbiError> {
    let mut plan = base(abi, sig);
    let mut gpr = 0u16;
    let mut fpr = 0u16;
    if sig.ret.is_aggregate() && sig.ret.size() > 16 {
        indirect_return(&mut plan, &sig.ret, 0);
        gpr = 1;
    } else if sig.ret.is_aggregate() {
        // Without a homogeneous float aggregate, an aggregate return of up to
        // two eightbytes comes back in one or two general registers.
        // Without a homogeneous float aggregate, an aggregate return of up to
        // two eightbytes comes back in one or two general registers.
        let mut parts = Vec::new();
        for word in 0..sig.ret.size().div_ceil(8) {
            let offset = word * 8;
            let size = sig.ret.size().saturating_sub(offset).min(8);
            let location = Location::Register(RegisterId {
                class: RegisterClass::Gpr,
                index: word as u16,
            });
            parts.push(ValuePart {
                source_offset: offset,
                size,
                location,
            });
            add_register(
                &mut plan,
                RegisterId {
                    class: RegisterClass::Gpr,
                    index: word as u16,
                },
                None,
            );
        }
        plan.return_value = ReturnPlan::Direct { parts };
    } else if !sig.ret.is_void() {
        direct_return(
            &mut plan,
            &sig.ret,
            if sig.ret.is_float() {
                RegisterClass::Fpr
            } else {
                RegisterClass::Gpr
            },
            0,
        );
    }
    for (index, ty) in sig.params.iter().enumerate() {
        if let Some(hfa) = homogeneous_float_count(ty)
            && fpr + hfa as u16 <= 8
        {
            let mut parts = Vec::new();
            let component = match ty {
                AbiType::Struct { fields, .. } => &fields[0].ty,
                _ => ty,
            };
            let size = component.size();
            for n in 0..hfa {
                let loc = Location::Register(RegisterId {
                    class: RegisterClass::Fpr,
                    index: fpr,
                });
                add_register(
                    &mut plan,
                    RegisterId {
                        class: RegisterClass::Fpr,
                        index: fpr,
                    },
                    Some((index, n * size, size)),
                );
                parts.push(ValuePart {
                    source_offset: n * size,
                    size,
                    location: loc,
                });
                fpr += 1;
            }
            plan.args.push(ArgPlan {
                index,
                ty: ty.clone(),
                passing: Passing::Direct { parts },
            });
            continue;
        }
        if ty.is_aggregate() && ty.size() <= 16 && gpr + ty.size().div_ceil(8) as u16 <= 8 {
            let mut parts = Vec::new();
            for word in 0..ty.size().div_ceil(8) {
                let offset = word * 8;
                let size = ty.size().saturating_sub(offset).min(8);
                parts.push(ValuePart {
                    source_offset: offset,
                    size,
                    location: Location::Register(RegisterId {
                        class: RegisterClass::Gpr,
                        index: gpr,
                    }),
                });
                add_register(
                    &mut plan,
                    RegisterId {
                        class: RegisterClass::Gpr,
                        index: gpr,
                    },
                    Some((index, offset, size)),
                );
                gpr += 1;
            }
            plan.args.push(ArgPlan {
                index,
                ty: ty.clone(),
                passing: Passing::Direct { parts },
            });
            continue;
        }
        let mut budget = RegisterBudget {
            gpr,
            fpr,
            max_gpr: 8,
            max_fpr: 8,
            indirect_threshold: 16,
            byval_aggregates: false,
        };
        scalar_or_indirect(&mut plan, index, ty, &mut budget)?;
        gpr = budget.gpr;
        fpr = budget.fpr;
    }
    Ok(plan)
}

/// RISC-V and LoongArch flatten small aggregates field by field instead of
/// classifying eightbytes: floating fields get their own FP register, integer
/// fields share a register while they fit in one XLEN. An aggregate that does
/// not fit is passed by reference.
fn riscv_like(abi: AbiKind, sig: &FunctionSignature) -> Result<CallPlan, AbiError> {
    let mut plan = base(abi, sig);
    let mut gpr = 0u16;
    let mut fpr = 0u16;

    if sig.ret.is_aggregate() {
        match aggregate_parts(&sig.ret) {
            Some(parts) if parts_fit(&parts, gpr, fpr, 2, 2) => {
                let mut emitted = Vec::new();
                for part in &parts {
                    let index = if part.class == RegisterClass::Gpr {
                        let i = gpr;
                        gpr += 1;
                        i
                    } else {
                        let i = fpr;
                        fpr += 1;
                        i
                    };
                    emitted.push(ValuePart {
                        source_offset: part.offset,
                        size: part.size,
                        location: Location::Register(RegisterId {
                            class: part.class,
                            index,
                        }),
                    });
                    add_register(
                        &mut plan,
                        RegisterId {
                            class: part.class,
                            index,
                        },
                        None,
                    );
                }
                plan.return_value = ReturnPlan::Direct { parts: emitted };
            }
            _ => indirect_return(&mut plan, &sig.ret, 0),
        }
    } else if !sig.ret.is_void() {
        let class = if sig.ret.is_float() {
            RegisterClass::Fpr
        } else {
            RegisterClass::Gpr
        };
        direct_return(&mut plan, &sig.ret, class, 0);
    }

    for (index, ty) in sig.params.iter().enumerate() {
        if ty.is_aggregate() {
            match aggregate_parts(ty) {
                Some(parts) if parts_fit(&parts, gpr, fpr, 8, 8) => {
                    let mut emitted = Vec::new();
                    for part in &parts {
                        let (class, index_reg) = if part.class == RegisterClass::Gpr {
                            let i = gpr;
                            gpr += 1;
                            (RegisterClass::Gpr, i)
                        } else {
                            let i = fpr;
                            fpr += 1;
                            (RegisterClass::Fpr, i)
                        };
                        emitted.push(ValuePart {
                            source_offset: part.offset,
                            size: part.size,
                            location: Location::Register(RegisterId {
                                class,
                                index: index_reg,
                            }),
                        });
                        add_register(
                            &mut plan,
                            RegisterId {
                                class,
                                index: index_reg,
                            },
                            Some((index, part.offset, part.size)),
                        );
                    }
                    plan.args.push(ArgPlan {
                        index,
                        ty: ty.clone(),
                        passing: Passing::Direct { parts: emitted },
                    });
                }
                _ => {
                    let location = if gpr < 8 {
                        let location = Location::Register(RegisterId {
                            class: RegisterClass::Gpr,
                            index: gpr,
                        });
                        add_register(
                            &mut plan,
                            RegisterId {
                                class: RegisterClass::Gpr,
                                index: gpr,
                            },
                            Some((index, 0, pointer_size(abi))),
                        );
                        gpr += 1;
                        location
                    } else {
                        add_stack(&mut plan, pointer_size(abi), 8, false)
                    };
                    plan.args.push(ArgPlan {
                        index,
                        ty: ty.clone(),
                        passing: Passing::Indirect {
                            pointer: location,
                            size: ty.size(),
                            align: ty.align(),
                            byval: false,
                        },
                    });
                }
            }
            continue;
        }
        let mut budget = RegisterBudget {
            gpr,
            fpr,
            max_gpr: 8,
            max_fpr: 8,
            indirect_threshold: 16,
            byval_aggregates: false,
        };
        scalar_or_indirect(&mut plan, index, ty, &mut budget)?;
        gpr = budget.gpr;
        fpr = budget.fpr;
    }
    Ok(plan)
}

fn pointer_size(abi: AbiKind) -> u32 {
    (abi.data_layout().pointer_bits as u32) / 8
}

/// One register-sized piece of a flattened aggregate.
struct Part {
    offset: u32,
    size: u32,
    class: RegisterClass,
}

fn parts_fit(parts: &[Part], gpr: u16, fpr: u16, max_gpr: u16, max_fpr: u16) -> bool {
    let need_g = parts
        .iter()
        .filter(|part| part.class == RegisterClass::Gpr)
        .count() as u16;
    let need_f = parts
        .iter()
        .filter(|part| part.class == RegisterClass::Fpr)
        .count() as u16;
    gpr + need_g <= max_gpr && fpr + need_f <= max_fpr
}

/// Flatten an aggregate into register pieces, or return `None` when it has to
/// be passed by reference.
fn aggregate_parts(ty: &AbiType) -> Option<Vec<Part>> {
    if ty.size() > 16 {
        return None;
    }
    // A homogeneous aggregate of one floating type keeps one register per
    // element, which is exactly what the field walk below produces.
    let mut leaves = Vec::new();
    collect_leaves(ty, 0, &mut leaves)?;
    if leaves.is_empty() {
        return None;
    }
    let mut parts: Vec<Part> = Vec::new();
    // Integer fields share a register while they fit in one XLEN.
    let mut pending: Option<Part> = None;
    for leaf in leaves {
        if leaf.class == RegisterClass::Fpr {
            if let Some(pending) = pending.take() {
                parts.push(pending);
            }
            parts.push(leaf);
            continue;
        }
        match &mut pending {
            Some(current)
                if current.offset + current.size + leaf.size <= 8
                    && leaf.offset == current.offset + current.size =>
            {
                current.size += leaf.size;
            }
            _ => {
                if let Some(pending) = pending.take() {
                    parts.push(pending);
                }
                pending = Some(leaf);
            }
        }
    }
    if let Some(pending) = pending {
        parts.push(pending);
    }
    Some(parts)
}

fn collect_leaves(ty: &AbiType, base: u32, out: &mut Vec<Part>) -> Option<()> {
    match ty {
        AbiType::Float { .. } => out.push(Part {
            offset: base,
            size: ty.size(),
            class: RegisterClass::Fpr,
        }),
        AbiType::Integer { .. } | AbiType::Pointer { .. } | AbiType::Bool => out.push(Part {
            offset: base,
            size: ty.size(),
            class: RegisterClass::Gpr,
        }),
        AbiType::Array { element, count } => {
            for index in 0..*count {
                collect_leaves(element, base + index * element.size(), out)?;
            }
        }
        AbiType::Struct { fields, .. } => {
            for field in fields {
                collect_leaves(&field.ty, base + field.offset, out)?;
            }
        }
        // A union cannot be flattened: its members overlap.
        AbiType::Union { fields, .. } => {
            let all_float = fields.iter().all(|field| field.ty.is_float());
            out.push(Part {
                offset: base,
                size: ty.size().min(8),
                class: if all_float {
                    RegisterClass::Fpr
                } else {
                    RegisterClass::Gpr
                },
            });
        }
        _ => return None,
    }
    Some(())
}

fn homogeneous_float_count(ty: &AbiType) -> Option<u32> {
    let AbiType::Struct { fields, .. } = ty else {
        return None;
    };
    if fields.is_empty() || fields.len() > 4 || !fields.iter().all(|f| f.ty.is_float()) {
        return None;
    }
    let bits = fields[0].ty.size();
    fields
        .iter()
        .all(|f| f.ty.size() == bits)
        .then_some(fields.len() as u32)
}

fn aggregate_classes(ty: &AbiType, eightbyte: u32) -> Vec<RegisterClass> {
    let count = ty.size().div_ceil(eightbyte).max(1);
    (0..count)
        .map(|n| {
            if aggregate_chunk_is_float(ty, n * eightbyte, eightbyte) {
                RegisterClass::Fpr
            } else {
                RegisterClass::Gpr
            }
        })
        .collect()
}

fn aggregate_chunk_is_float(ty: &AbiType, start: u32, width: u32) -> bool {
    match ty {
        AbiType::Float { .. } => true,
        AbiType::Struct { fields, .. } | AbiType::Union { fields, .. } => {
            let end = start + width;
            let mut found = false;
            for field in fields {
                let field_end = field.offset + field.ty.size();
                if field.offset < end && field_end > start {
                    if !field.ty.is_float() {
                        return false;
                    }
                    found = true;
                }
            }
            found
        }
        _ => false,
    }
}
