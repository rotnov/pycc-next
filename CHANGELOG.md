# Changelog

All notable changes to pycc-next. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Changed
- Type-checker diagnostics (`T0xxx`, and the `C0001`/`I0404` the checker raises) point
  at the offending expression or statement instead of `1:1`, with a short caret label
  (`expected `int`, found `str``) instead of the headline repeated; `--error-format
  json` carries the same line, column and length (#10).
- `--non-strict` native builds infer unannotated function signatures from their call
  sites, and keep imports and `sys` stream aliases compiled when fallback code reads
  them: 72% of the corpus's top-level statements now compile natively (was 51%) (#11).

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

- Reference counting for `str`, lists, dicts, sets, class instances and CPython
  objects; no per-iteration leaks; cycles are not collected (see DEVIATIONS).
- Empty-container inference, closures (`nonlocal` via cells), `lambda`, `key=` /
  `reverse=` for `sorted`/`min`/`max`, set operators, `math` integer functions.
- Shared stdin/stdout with CPython when it is embedded or hosting an `--ext` module.

### Security
- `str` hashing is CPython's keyed SipHash-1-3 (`PYTHONHASHSEED` honoured).
- `int(str)` enforces CPython's 4300-digit limit.
- Big ints passed into `--ext` modules raise instead of aborting the host.
- Fixed use-after-free / double-free classes found by review (operand pinning,
  set in-place ops, deep frees without recursion).

### Performance
- Inline small-int arithmetic, direct calls, inline exception checks, fixed-layout
  class instances, inline float `/` and `**`, one-byte `list[bool]`.
- vs CPython 3.13 (bench/run.py): mandelbrot ~40x, spectral_norm ~15x, fib ~12x,
  sieve ~6x, nbody ~5-25x faster; mandelbrot runs at hand-written Rust speed.

### Real-world corpus (200 competitive-programming solutions)
- `--non-strict`: 200/200 match CPython (CI-gated, 0 wrong outputs).
- strict: 93/200 compile and match CPython unchanged (most of the rest only lack
  annotations on public functions, which strict mode requires by design).
- `--non-strict`: 51% of top-level statements compile natively, the rest fall back
  to CPython per statement.

### Known gaps
- See `docs/DEVIATIONS.md` for every known difference from CPython 3.13.
- Linux x86_64 only for prebuilt binaries.
