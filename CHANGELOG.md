# Changelog

All notable changes to pycc-next. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added

- The generator protocol, natively in strict mode: `send()`, `throw()`,
  `close()` (`GeneratorExit`, "generator ignored GeneratorExit"), `yield` as
  an expression, `return value` as `StopIteration.value`, `yield from`
  delegating all three and returning the subgenerator's value, PEP 479, and
  generators in module globals closed by `del` and at exit.
- Full call signatures: `*args`, keyword-only parameters, `**kwargs`,
  positional-only parameters and defaults of every kind, for functions,
  methods, constructors and nested functions, called with keywords, `*seq`
  and `**mapping`, with CPython's `TypeError` messages.
- Function values (module-level functions and closures) stored in lists,
  dicts and instance attributes and called from there (`fs[0](x)`,
  `table[k](x)`, `self.f(x)`); `print`, `str()` and f-strings render a
  function or a generator as CPython does (`<function f at 0x...>`,
  `<generator object g at 0x...>`).
- `--ext`: a public function or method with defaults, positional-only or
  keyword-only parameters, `*args` or `**kwargs` is published to Python
  callers with its declared signature (#1194); CPython binds the call and
  raises its own `TypeError`s.

### Performance
- Every benchmark is within 3x of its Rust twin (fib 1.8x, mandelbrot 1.1x, nbody 2.4x,
  sieve 2.1x, spectral_norm 1.8x; v0.1: up to 4.2x). Whole-program proofs drop checks that
  cannot fire: an attribute every constructor assigns before the instance escapes skips the
  "assigned" test; a function that cannot raise is called without the exception check; a
  `def` that precedes all module code skips the call-before-definition check. A tree of
  `int` operators tests its leaves' tags once; `x += y` releases the old value only on the
  bigint path; range loops with one level of nested loops count with a machine integer;
  `x ** 0.5` proves `pow == sqrt` with exact integer arithmetic.
- `bench/run.py`: best of 7 by default; Rust twins build for the default target CPU and
  black-box every input.
- `PYCC_DUMP_LLVM=<path>` writes the LLVM IR before and after optimization.

## [0.1.0] - 2026-10-11

First release of pycc-next: an ahead-of-time compiler for typed Python that turns
type mismatches into compile errors and runs 11-34x faster than CPython on the
benchmark set, interoperating with CPython through extension modules and an
embedded interpreter. Everything below in 0.1.0-rc.1 and 0.1.0-rc.2 is included.

Highlights:
- Strict mode (default): Rust-style diagnostics at the offending code; native
  representations; generators, closures, classes, exceptions, containers, bigints.
- `--non-strict`: untyped code compiles; unprovable parts run through CPython,
  per statement if needed. 200/200 real-world corpus programs match CPython.
- `--ext`: abi3 CPython extension modules (CPython >= 3.13).
- Reference-counted memory with no per-iteration leaks; security review fixes
  (keyed str hashing, int(str) digit limit, no host aborts on big ints).
- Benchmarks vs CPython 3.13 / hand-written Rust: fib 10.8x / 3.8x slower,
  mandelbrot 33.7x / 1.1x, nbody 23.3x / 4.2x, sieve 16.6x / 3.1x,
  spectral_norm 29.2x / 3.5x (see README).
- Self-contained Linux x86_64 release archive (needs `cc`; CPython 3.13+ with headers
  for `--ext` and `--non-strict`).

Known differences from CPython: docs/DEVIATIONS.md.

## [0.1.0-rc.2] - 2026-10-11

The release archive is now self-contained (rc.1 could not build programs). Changes
since rc.1:

### Added
- `global` statements: a module global a function rebinds lives in a one-element
  cell, so every function and the module see its writes (native in strict mode).
- `def`s inside module-level blocks (`for _ in range(t): def solve(): ...`) compile
  natively when used only after their definition in that block.
- Comprehensions with several `for`/`if` clauses, and starred call arguments
  (`f(*xs)`, `f(a, *t)`) with CPython's `TypeError` for a wrong count.
- Instance attributes initialised in `__init__` from any expression
  (`self.t = [0] * (n + 1)`): the slot takes the value's inferred type.
- `len()` of a tuple bound to a name.
- Generators, native in strict mode: `def`s with `yield` (annotated `Iterator[T]`,
  `Iterable[T]` or `Generator[T, None, None]`, or inferred for a private helper),
  `yield from` statements, `return` in a generator, `for`/`next()`/`list()`/`sum()`/...
  over them, and generator expressions, lazy (so `any()`/`all()` stop at the first
  deciding element). An unfinished generator is closed when its last reference goes,
  running its `finally` clauses. An `--ext` module's public generator returns a
  CPython iterator.
- Function values (`Callable[[A, B], R]`): module-level functions used as values and
  nested functions returned or passed as closures (capturing their enclosing variables,
  `nonlocal` counters included), called through a variable or a call's result
  (`make_adder(1)(2)`). A closure binds a variable when it is made, so one that reads
  a variable the enclosing function may still change stays unsupported.

### Fixed
- A heap `int` (beyond 63 bits) held by a function's parameters or locals is released
  when the function returns, and one passed to a builtin (`str(x * 2)`,
  `max(x, x + 1)`) after the call: big-int loops through functions run in constant
  memory (#625).
- Function values: a call keeps the called closure alive while its arguments run
  (an argument rebinding the variable that held it no longer frees it under the
  call); a closure capturing a variable rebound by `except ... as`, a `match`
  capture, a `type` alias or a comprehension walrus, or used before its `def`, is
  refused (strict) or runs in CPython (non-strict) instead of seeing a stale or
  unassigned value; the compiler's internal names (`__pycc_closure__`, `_pycc_*`)
  are reserved (`E0109`). A `--non-strict` fallback function annotated with a
  compiled `typing` import (`Callable`) no longer fails with `NameError`.
- The release archive is self-contained and relocatable (#5): it ships
  `bin/pycc` + `lib/pycc/libpycc_rt.a`, and `pycc` finds its runtime relative to
  its own executable instead of the checkout it was built in (`v0.1.0-rc.1` failed
  with "no pycc_rt build found (expected /home/runner/work/...)"). The release
  workflow smoke-tests the unpacked tarball (native, `--ext`, `--non-strict`)
  before publishing. A missing C toolchain is reported with what to install.
- `pycc version` reports the LLVM it was built against (from `llvm-config`)
  instead of a hard-coded `22.1.1`.

### Changed
- Type-checker diagnostics (`T0xxx`, and the `C0001`/`I0404` the checker raises) point
  at the offending expression or statement instead of `1:1`, with a short caret label
  (`expected `int`, found `str``) instead of the headline repeated; `--error-format
  json` carries the same line, column and length (#10).
- `--non-strict` native builds infer unannotated function signatures from their call
  sites, keep imports and `sys` stream aliases compiled when fallback code reads
  them, call CPython's builtin for a builtin call they do not compile, and keep
  scalar globals that fallback code only reads native: 83% of the corpus's top-level
  statements now compile natively (was 51%) (#11).
- `--non-strict`: fallback code calling a compiled function with a parameter default
  left out raised `TypeError`; such a function now runs in CPython.

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
