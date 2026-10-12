---
name: bench
description: Use when measuring pycc-next performance, after any codegen/runtime change that could affect speed, or before quoting a speed number anywhere (README, blog, release notes).
---

# Benchmarks

1. `cargo build --release` (export `LLVM_SYS_221_PREFIX=/usr/lib/llvm-22`).
2. `python3 bench/run.py` (all programs) or `python3 bench/run.py fib nbody`.
   It compiles each `bench/py/*.py` with pycc-next, checks output equals CPython, and times
   CPython, pycc-next and `bench/rs/*.rs` (`rustc -O`, default target CPU like pycc's own
   output -- no `target-cpu=native`). Best of 7.
3. A row with `OUTPUT MISMATCH` or `compile error` is a bug, not a footnote.
4. New program: add `bench/py/<name>.py` (idiomatic typed Python, prints a checksum) and the
   same algorithm in `bench/rs/<name>.rs`. Keep the work identical; use
   `std::hint::black_box` on inputs so rustc can't constant-fold.
5. Quote numbers only from a fresh run on the current commit; say which machine.
   Report slowdowns as prominently as speedups.
6. To see why something is slow: `objdump -d --no-show-raw-insn <binary>` and look for
   runtime calls (`pycc_rt_*`) in hot functions.
