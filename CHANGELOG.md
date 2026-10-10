# Changelog

All notable changes to pycc-next. Format: [Keep a Changelog](https://keepachangelog.com/).

## [0.1.0-rc.1] - 2026-10-10

First release candidate of the pycc restart: compile typed Python to native code,
catch type errors at compile time, interoperate with CPython.

### Added
- Compiler pipeline imported from pycc@aab8cc61: parser, HIR, strict type checker,
  MIR, LLVM 22 codegen, native runtime, Rust-style diagnostics.
- Build modes: native executables, CPython extension modules (`--ext`, abi3,
  CPython >= 3.13), and native executables that embed the installed CPython (>= 3.13)
  when the program uses CPython objects.
- `--non-strict` mode (or `[tool.pycc] strict = false`): untyped code compiles; values
  that can't be proven typed run through the CPython C API; constructs the compiler
  can't lower yet fall back per function / top-level block to CPython
  (`--report-fallbacks`).
- `break`, `continue`, `while ... else`, `for ... else` with path-precise
  definite-assignment analysis.
- Generic `list[T]` (int, float, bool, str, class instances, nested lists), `[x] * n`,
  slicing, `round()`, `input()`, common `str` methods and builtins.
- Arbitrary-precision `int` arithmetic with inline small-int fast paths.
- Benchmarks against CPython and hand-written Rust (`bench/run.py`) and a
  real-world corpus metric (`bench/corpus.py`).

### Performance
- Inline small-int arithmetic, direct calls, inline exception checks, fixed-layout
  class instances, inline float `/` and `**`, one-byte `list[bool]`.

### Known gaps
- See `docs/DEVIATIONS.md` for every known difference from CPython 3.13.
- Linux x86_64 only for prebuilt binaries.
