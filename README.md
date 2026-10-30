# abi-plan

`abi-plan` converts a C function signature into a target ABI call plan. It does
not emit machine code. A caller, simulator, JIT, or compiler backend executes
the plan by moving each source byte range to the listed register or stack
location.

```text
AbiType + FunctionSignature + AbiKind
                |
                v
             CallPlan
```

The public model keeps source object layout separate from transport layout.
Struct fields carry their source offsets; a `ValuePart` carries a source byte
range and its destination `Location`.

The crate currently includes plans for:

- Shy ABI v1: eight 32-bit argument slots, sret in the first slot, aggregate
  arguments on the stack, and the project's upward-growing stack convention.
- x86-64 SysV: common scalar and small aggregate INTEGER/SSE plans.
- Windows x64: positional argument slots (an argument takes RCX/RDX/R8/R9 or
  XMM0..XMM3 by position, so the fifth argument is on the stack even when
  registers are free), 32-byte shadow space, aggregates of 1/2/4/8 bytes by
  value, and larger aggregates by reference.
- AArch64, RISC-V64, and LoongArch64: common scalar, HFA, small aggregate,
  and indirect aggregate cases.

The mainstream classifiers intentionally return an error for ABI extensions
that the model cannot represent faithfully. They are a reusable base for a
full dynamic-library bridge; callers must still verify platform-specific
edge cases such as vectors, `long double`, packed records, and varargs.
