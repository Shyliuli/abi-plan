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
- Windows x64: four register slots, 32-byte shadow space, and indirect large
  aggregates.
- AArch64, RISC-V64, and LoongArch64: common scalar, HFA, small aggregate,
  and indirect aggregate cases.

The mainstream classifiers intentionally return an error for ABI extensions
that the model cannot represent faithfully. They are a reusable base for a
full dynamic-library bridge; callers must still verify platform-specific
edge cases such as vectors, `long double`, packed records, and varargs.
