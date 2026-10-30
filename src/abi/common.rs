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
    let mut gpr = 0u16;
    let mut fpr = 0u16;
    if sig.ret.is_aggregate() && sig.ret.size() > 8 {
        indirect_return(&mut plan, &sig.ret, 0);
        gpr = 1;
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
        if ty.is_aggregate() && !matches!(ty.size(), 1 | 2 | 4 | 8) {
            if gpr < 4 {
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
            } else {
                let loc = add_stack(&mut plan, 8, 8, false);
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
            }
            continue;
        }
        if ty.is_float() && fpr < 4 {
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
                Some((index, 0, ty.size())),
            );
            fpr += 1;
            plan.args.push(ArgPlan {
                index,
                ty: ty.clone(),
                passing: Passing::Direct {
                    parts: scalar_parts(ty, loc, 0),
                },
            });
        } else if gpr < 4 {
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
                Some((index, 0, ty.size().min(8))),
            );
            gpr += 1;
            plan.args.push(ArgPlan {
                index,
                ty: ty.clone(),
                passing: Passing::Direct {
                    parts: scalar_parts(ty, loc, 0),
                },
            });
        } else {
            let loc = add_stack(&mut plan, align_up(ty.size().max(8), 8), 8, false);
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
    }
    Ok(plan)
}

pub fn aapcs64(sig: &FunctionSignature) -> Result<CallPlan, AbiError> {
    arm_like(AbiKind::Aapcs64, sig)
}

pub fn riscv64(sig: &FunctionSignature) -> Result<CallPlan, AbiError> {
    arm_like(AbiKind::Riscv64, sig)
}

pub fn loongarch64(sig: &FunctionSignature) -> Result<CallPlan, AbiError> {
    arm_like(AbiKind::LoongArch64, sig)
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
        let mut budget = RegisterBudget {
            gpr,
            fpr,
            max_gpr: 8,
            max_fpr: 8,
            indirect_threshold: 16,
        };
        scalar_or_indirect(&mut plan, index, ty, &mut budget)?;
        gpr = budget.gpr;
        fpr = budget.fpr;
    }
    Ok(plan)
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
