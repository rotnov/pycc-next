---
name: bench
description: Use when measuring pycc-next performance, after any codegen/runtime change that could affect speed, or before quoting a speed number anywhere (README, blog, release notes).
---

# Benchmarks

1. `cargo build --release` (export `LLVM_SYS_221_PREFIX=/usr/lib/llvm-22`).
2. `python3 bench/run.py` (all programs) or `python3 bench/run.py fib nbody`.
   It compiles each `bench/py/*.py` with pycc-next, checks output equals CPython, and times
   CPython, pycc-next and `bench/rs/*.rs` (`rustc -O`, default target CPU like pycc's own
   output -- no `target-cpu=native`): `--runs` (default 11) rounds of fresh processes, the
   three sides interleaved; the table shows medians and min-max, `--json PATH` writes every
   sample. `bench/package_check.py PKG --time` and `bench/pydantic_check.py --time` time the
   real packages the same way (interleaved fresh processes, median and spread, `--json`).
3. A row with `OUTPUT MISMATCH` or `compile error` is a bug, not a footnote.
4. New program: add `bench/py/<name>.py` (idiomatic typed Python, prints a checksum) and the
   same algorithm in `bench/rs/<name>.rs`. Keep the work identical; use
   `std::hint::black_box` on inputs so rustc can't constant-fold.
5. **Published numbers come from the `bench` workflow** (`.github/workflows/bench.yml`:
   GitHub-hosted ubuntu-24.04 and ubuntu-24.04-arm runners; on release tags, weekly, and
   `gh workflow run bench.yml --ref main`). It runs `bench/run.py`, the five packages'
   `--time` checks and `bench/pydantic_check.py --time`, and publishes medians with their
   spread, the runner's CPU model and the number of runs, in the job summary and the
   `bench-<os>` artifact (`bench.json`, `bench.md`; `bench/publish.py`). Every speed number
   in README, CHANGELOG, release notes and blog posts cites such a run by its run id (and
   the runner's CPU from its summary); copy numbers from it, never from a local run. Report
   slowdowns as prominently as speedups, and a ratio inside the spread as "about equal".
6. Local runs are for deciding what to change, on the current commit. To see why something
   is slow: `objdump -d --no-show-raw-insn <binary>` and look for runtime calls
   (`pycc_rt_*`) in hot functions.
7. The machine is shared: decide with instruction counts
   (`valgrind --tool=callgrind <binary>`; `bench/package_check.py --instructions`, whose
   bounds are CI's regression gate in `ci.yml`), not local wall time.
   `PYCC_DUMP_LLVM=<path> pycc build ...` writes the IR before (`<path>.pre.ll`) and after
   (`<path>`) optimization; `opt -passes='default<O3>'` on the `.pre.ll` reproduces a release
   build for pipeline experiments.
