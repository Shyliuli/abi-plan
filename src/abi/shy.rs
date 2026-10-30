//! ShyProject ABI v1 classifier.
use super::*;

pub fn plan(sig: &FunctionSignature) -> Result<CallPlan, AbiError> {
    let mut plan = base(AbiKind::ShyV1, sig);
    let mut slot: u16 = 0;
    let mut stack = 0u32;

    if sig.ret.is_aggregate() {
        plan.return_value = ReturnPlan::Indirect {
            pointer: Location::Register(RegisterId {
                class: RegisterClass::Gpr,
                index: 0,
            }),
            size: sig.ret.size(),
            align: sig.ret.align(),
        };
        add_register(
            &mut plan,
            RegisterId {
                class: RegisterClass::Gpr,
                index: 0,
            },
            None,
        );
        slot = 1;
    } else if sig.ret.is_void() {
        plan.return_value = ReturnPlan::Void;
    } else if sig.ret.size() <= 4 {
        direct_return(&mut plan, &sig.ret, RegisterClass::Gpr, 1);
    } else if sig.ret.size() <= 8 {
        plan.return_value = ReturnPlan::Direct {
            parts: vec![
                ValuePart {
                    source_offset: 4,
                    size: 4,
                    location: Location::Register(RegisterId {
                        class: RegisterClass::Gpr,
                        index: 1,
                    }),
                },
                ValuePart {
                    source_offset: 0,
                    size: 4,
                    location: Location::Register(RegisterId {
                        class: RegisterClass::Gpr,
                        index: 2,
                    }),
                },
            ],
        };
    } else {
        return Err(AbiError::Unsupported {
            abi: AbiKind::ShyV1,
            message: "scalar return wider than 64 bits".into(),
        });
    }

    for (index, ty) in sig.params.iter().enumerate() {
        let size = ty.size();
        let aggregate = ty.is_aggregate() || matches!(ty, AbiType::Opaque { .. });
        if !aggregate && size > 8 {
            return Err(AbiError::Unsupported {
                abi: AbiKind::ShyV1,
                message: format!("argument {index} is wider than 64 bits"),
            });
        }
        let needs = if !aggregate && size > 4 { 2 } else { 1 };
        if !aggregate && slot + needs <= 8 {
            let mut parts = Vec::new();
            if size > 4 {
                // Shy register pairs are low-word first; C object bytes are big-endian.
                parts.push(ValuePart {
                    source_offset: 4,
                    size: 4,
                    location: Location::Register(RegisterId {
                        class: RegisterClass::Gpr,
                        index: slot,
                    }),
                });
                parts.push(ValuePart {
                    source_offset: 0,
                    size: 4,
                    location: Location::Register(RegisterId {
                        class: RegisterClass::Gpr,
                        index: slot + 1,
                    }),
                });
            } else {
                parts.push(ValuePart {
                    source_offset: 0,
                    size,
                    location: Location::Register(RegisterId {
                        class: RegisterClass::Gpr,
                        index: slot,
                    }),
                });
            }
            for (part_index, part) in parts.iter().enumerate() {
                add_register(
                    &mut plan,
                    RegisterId {
                        class: RegisterClass::Gpr,
                        index: slot + part_index as u16,
                    },
                    Some((index, part.source_offset, part.size)),
                );
            }
            plan.args.push(ArgPlan {
                index,
                ty: ty.clone(),
                passing: Passing::Direct { parts },
            });
            slot += needs;
        } else {
            let block = align_up(size.max(4), 4);
            stack = align_up(stack, 4);
            let location = Location::Stack {
                offset: -((stack + block) as i32),
            };
            let parts = vec![ValuePart {
                source_offset: 0,
                size,
                location,
            }];
            plan.args.push(ArgPlan {
                index,
                ty: ty.clone(),
                passing: Passing::Direct { parts },
            });
            stack += block;
        }
    }
    plan.stack.argument_bytes = stack;
    Ok(plan)
}
