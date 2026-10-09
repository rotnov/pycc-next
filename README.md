# pycc-next

Ahead-of-time compiler for type-annotated Python. Restart of
[pycc](https://github.com/rotnov/pycc) with the same goal and a lean process.

- **Strict typing:** type mismatches are compile errors, not runtime `TypeError`s.
- **Speed:** typed code gets native representations (`i64`, `f64`, structs, buffers).
- **Interop:** output can be a CPython extension module that calls numpy, torch, etc.

Devlog: https://rotnov.github.io/pycc-next/

## Build

Requires Rust 1.97.1 and LLVM 22.

```console
$ export LLVM_SYS_221_PREFIX=/usr/lib/llvm-22
$ cargo build --release
$ ./target/release/pycc build hello.py -o hello && ./hello
```

## Status

Compiler crates imported from pycc@aab8cc6 (parser, HIR, type checker, MIR, LLVM
codegen, runtime). See the devlog for progress.

## License

MIT
