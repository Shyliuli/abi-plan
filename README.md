# abi-plan

`abi-plan` converts a C function signature into a target ABI call plan. It is being rewritten by hand.

## WIP
- x86_64 SysV ABI

## Tests

Each file in `test/cases` is one C function. Its file stem is the function
name, and its metadata is written in comments. `result` defaults to `int`.

```c
// args: int
// result: int
// input: 42
// expected: 42
```

Run the framework from the repository root:

```sh
cargo run-test
```

Pass/fail counts and detailed failures are written to
`target/test-report.txt`. The suite deliberately includes signatures that the
current rewrite does not support, so those cases remain failures until their
ABI handling is implemented.

## Goal
- x86_64 Windows ABI
- RISCV
- LoongArch
- Aarch64
- ShyISA
- ....
