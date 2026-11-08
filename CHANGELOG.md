# Changelog

All notable changes to pycc-next. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- `pycc profile PKG -o pycc.profile.json -- PYTHON SCRIPT [ARGS]`
  (`docs/design/compile-little.md`, section 2.2): runs a workload against a package's
  sources, nothing compiled, and writes one JSON file with, per function of the package,
  its calls, self time and calls to Python functions by callee (medians over `--runs`
  fresh `cProfile` runs) and the SHA-256 of its source; the functions the selection rule
  (`--select-share`, `--select-calls`, `--select-max`, and `--select-outside`: less
  than that share of a function's Python calls may leave the selected region, section 2.3)
  picks also get the exact types of their arguments in their first 1,000
  calls (`sys.monitoring`). Entries are sorted, so a re-profile diffs cleanly; `--check`
  reads a profile against the current sources and prints its stale entries and the
  selection. `bench/pydantic_workload.py --train` is the training input: the same phases
  over other models.
- `bench/pydantic_check.py --cachegrind [PYTHON]`: pydantic's instructions, I1 misses and
  branch mispredictions per phase of the workload (each phase alone, define and schema
  together, and the whole process) under `valgrind --tool=callgrind --cache-sim=yes
  --branch-sim=yes`, compiled tree against CPython, with the extension's `.so` and `.text`
  size; the CI benchmark workflow publishes them. The per-function harness behind
  `docs/design/compile-little.md`'s section 1 moved to `bench/headroom/`.
- `bench/pydantic_check.py --time` (and the published CI benchmark) reports `process`
  alongside the workload's phases: each run's whole-process wall time, from starting the
  interpreter to its exit.

### Changed
- A `--release` module's object release is split: its `NULL` and immortality tests are
  inlined at each site, and only a mortal object's decrement and `_Py_Dealloc` stay one
  call into the module's own `pycc_ext_obj_release_count`. Most releases in a generic body
  are of an empty slot, a cached one-character `str`, a small `int` or `None`. They now
  cost a test and a branch instead of a call. tomli `loads()`: 23.77M -> 22.81M
  instructions.
- Operations on CPython objects in generic bodies (attribute get, method and plain calls,
  `len`, subscripts, unpacking, comparisons, arithmetic, displays, `isinstance`, `in`,
  slices, boxing and unboxing) no longer test the thread's pending exception after the
  statement that runs them: each emitter already routes a NULL or negative result to the
  failure edge at once (`crates/pycc_codegen/src/foreign_fail.rs`), so the test was dead.
  Dropping it shrinks pydantic's compiled `.text` by about 7% (4.14 MB to 3.83 MB with
  `--select all`), its instructions by up to 3.7% and its I1 misses by 2 to 4%.
  `tests/object_failure_edges.rs` checks every failing object operation against CPython,
  with and without the inline fast paths.
- A `str` that crosses from CPython into compiled code (an `--ext` argument, a value
  unboxed from an object) starts with its code-point count, which CPython keeps, so
  `len(s)`, `s[i]` and the ASCII test on it no longer count the text once per crossing:
  a function taking a long `str` was O(n) per call before its first index. Native `str`
  indexing with a small index into ASCII text, comparison of the same object or of two
  one-byte texts, a counted `len`, and the counts of immortal strings run inline in a
  `--release` build
  (`crates/pycc_codegen/src/str_inline.rs`) instead of calling the runtime; `PyStrObj` is
  `repr(C)` with its text pointer, byte length and code-point count at fixed offsets.
- Generic bodies (non-strict `--ext`, every value a CPython object) run inline paths for
  exact `int`s and `str`s at their operations, as CPython's specializing interpreter does:
  compact-`int` `+`, `-`, `&`, `|`, `^` and comparisons, `str[int]` through the cached
  one-character strings, `bytes`/`bytearray` item reads and `bytearray` item stores, `str == "lit"` and `str == str` quick answers, `str in set` (a one-character ASCII
  `str` in a `frozenset` through a cached bit mask of the set), exact-class
  `isinstance` against a builtin, and a per-site cache for module globals
  (`docs/design/compile-little.md`, section 2.5).
  Anything else takes the helper call as before; `PYCC_NO_FAST_OPS=1` turns them off.
- A keyword method call from compiled code (`self.m(x, key=y)`) loads the method without
  making a bound method, and a call whose keywords name the parameters in order goes
  straight to the compiled body behind the method's forwarder.
- A call between compiled generic bodies (a module global or a method that names one
  compiled function, its keywords in parameter order) calls the body directly, without
  the forwarder: an inline check that the callee is still that function with its own
  `__code__`, then CPython's recursion check (`docs/design/compile-little.md`, sections
  2.5 and 2.8). A rebound name, a replaced `__code__`, an override, `*args` or a default
  left out take the call as before. `isinstance` against a builtin also answers `False`
  inline when the object is exactly another builtin, and packing an object argument is an
  inline reference increment.
- `a, b = f(...)` in a generic body, where `f` names one compiled function whose
  `return x, y` displays all have that many items, builds no tuple: the direct call runs
  a twin of `f` that stores the items after the first through out-pointers. Any other
  result of `f` (another return, a rebound `f`, a replaced `__code__`) is unpacked at the
  call site as before (`docs/design/compile-little.md`, section 2.5). Lifted bodies no
  longer get unused strict-mode multi-value twins.
- `x += 1` (and `x = x + y`, `-`, `&`, `|`, `^`) on an object local in a generic body
  writes the result into `x`'s `int` when the local holds its only reference, the result
  is a compact `int` and not one of CPython's cached small ints, instead of making a new
  `int` and freeing the old one (`docs/design/compile-little.md`, section 2.5).
- A non-strict `--ext` module's CPython code (its whole source, which runs as bytecode,
  and each published signature's wrapper) is compiled once at build time by the target
  interpreter and embedded as one marshalled tuple of code objects, which the import
  unmarshals as CPython loads a `.pyc` (`src/ext_build/module_code.rs`). The fallback
  chunk tables, the dataclass texts and forward references compiled at build time, the
  `def`s and signature wrappers made without running their code, and the `frozenset`
  order fixes are gone with the fallback chunks they served
  (`docs/design/compile-little.md`, section 7 items 1-5).
- **Non-strict `--ext` builds compile little** (`docs/design/compile-little.md`,
  sections 2.1-2.4): a module runs its source as CPython bytecode, and only its
  *selected* functions are compiled, each patched into the function object CPython
  made (`__pycc_patch__`), so the module's objects, `__file__` (its `.py`, which now
  ships beside the extension) and spec are CPython's. The selection comes from a
  profile (`--profile pycc.profile.json`, or `[tool.pycc] profile`) and the selection
  rule, adjusted by `--compile`/`--no-compile` (`[tool.pycc] compile`/`exclude`,
  `MODULE:QUALNAME`); `--select all` compiles every eligible function, `--select none`
  none. **With no profile nothing is selected**, and a module or package with nothing
  selected gets no extension at all: CPython imports it unchanged (a module built to `-o OUT`
  gets its source as `OUT`'s `.py`). A package's finder
  serves only its modules with selected functions. The whole-module compilation of
  before (`--select module`) is gone: the tests build with `--select all`, CI's package
  checks with a profile of each package's own suite (`--instructions` with `--select
  all`, whose counts do not depend on pass 3's timings). The export twins and the
  `--ext` branches of the fallback loop that only it used are deleted. The guessed
  specializations of unannotated functions, which only it produced, are no longer made
  (their `DEVIATIONS` row is gone; their code is section 7 item 19); a `range()` argument beyond 2**61 in a selected function raises the compiled
  range's own `OverflowError` message. The selection rule also leaves out a function that
  catches an exception in half of its calls or more (`--select-handled`; `pycc profile` records
  `handled`). **Only measured winners are selected**: `pycc profile` now builds the
  rule's candidates and times the workload with them compiled against the sources
  (pass 3; `--verify-rounds`, `--no-verify`, `--verify-instructions`), records the
  verdict on each (`measured`), and rejects a function measured slower ("slower when
  compiled") until its source changes; mistune's emphasis functions, for one, are
  rejected so. `pycc.build` accepts `[tool.pycc] select`, `profile`, `compile` and
  `exclude`, and ships a selected module's `.py` with its extension.
  `bench/pydantic_check.py` builds with the training profile by default, and
  `bench/package_check.py --select profile` profiles each package's own test suite.
- Three of an `--ext` module's object operations do less work for exact built-in types.
  An `int` index into an exact `tuple` or `list` reads the size and the item where CPython
  3.13 and 3.15 keep them (a 3.15 tuple's items after its cached hash; probed once),
  instead of calling the stable-ABI accessors.
  `isinstance` of a built-in class answers an instance of a plain built-in type without
  `PyType_IsSubtype`. The standard-library functions that compiled code calls are found
  by their call site's names in a hashed table, not by scanning it. pydantic's schema
  phase runs about 1.0M fewer instructions per repetition (109.8M -> 108.9M), and its
  definition phase about 1.4M fewer (30.9M -> 29.5M).
- A method call site's cache in an `--ext` module also holds a method of a class whose
  instances keep their values inline (a class CPython creates, such as one whose statement
  names a metaclass), while the class's entry says no instance attribute can shadow it:
  the site keeps the shared keys' count and checks it, with the instance's values inline,
  on every hit. A compiled method behind its Python forwarder is then called through the
  forwarder's slot, as its trampoline would, with no vectorcall through the trampoline;
  a keyword-enabled method's trampoline calls its compiled body directly too, and the
  receiver and arguments are copied without an inline `rep movsq`. pydantic's warm schema
  phase takes about 4% less time (14.87 -> 14.24 ms median, 21 interleaved runs) and its
  define phase about 2% less (7.32 -> 7.18 ms).
- An `--ext` module's published class whose statement's bases are none, `object`, or
  compiled classes of its module made the same way is made once, when its statement
  runs, straight from its compiled layout, methods and slots: no template type is made
  at import and none stands between the class and its bases, so its `__mro__` is
  CPython's with only the root `pycc.compiled` added before `object`. Other statements
  (a `Generic[T]` or other Python base, a metaclass, a keyword) still make the class
  from a template (`docs/DEVIATIONS.md`). The class deallocates its instances itself,
  running a `__del__` Python code assigns to it as `subtype_dealloc` did. pystache's
  import (16 classes made once) runs about 0.83M fewer instructions and 6 fewer page
  faults, pydantic's 0.74M fewer and 9 fewer faults, mistune's 0.15M fewer.
- An `--ext` module's import-time tables of names and sources (its interface names,
  published class names, builtin exception names, embedded sources and their
  file names) are one string each indexed by offset, and its forward
  trampolines are picked by a `switch`, instead of arrays of pointers: each pointer
  was a dynamic relocation, and every page holding one is copied when the extension
  is loaded. Relocated pages: pydantic 81 -> 66, mistune 41 -> 35, pystache 20 -> 18;
  page faults while loading the extension: pydantic 86 -> 71, mistune 46 -> 40,
  pystache 25 -> 23.
- The line an `--ext` module's fallback code runs after a statement to patch its
  compiled methods' forwarders names each one by its owner path and name, split at
  build time (`("C",), "m"`), instead of a dotted name split, sliced and looked up
  at import: the patching costs pydantic's import 1.05M -> 0.58M instructions.
- On CPython 3.15 an `--ext` module's namespace dict is watched only once its body has
  run: until then every compiled read of a global looks the name up, so the body's own
  stores no longer each fire the dict watcher (about 2,700 events in pydantic's
  import). Reads see every rebinding as before.
- Making an `--ext` module's published class checks the template's entries against
  its skip list with one `frozenset` lookup, and its slot descriptors by type, instead
  of string comparisons; a package's finder builds a member's path without splitting
  and joining its dotted name and calls `ModuleSpec` with fixed keyword names:
  pystache's import runs about 0.16M fewer instructions, pydantic's 0.1M.
- An `--ext` module's attribute-name cache and attribute inline caches keep their
  entries in the order they are first filled, behind a small index by hash, instead of
  at the hashed slot of a large zero-filled table: the import touches a few consecutive
  pages of them instead of most pages, each a page fault. Minor faults of the import:
  pydantic 1349 -> 1249, mistune 382 -> 308, pystache 186 -> 149.
- A `str` literal an `--ext` module only hands to CPython (a call's argument, a stored
  attribute, a subscript key) is made as a CPython `str` directly, on the first
  evaluation of the module's sites of that text, instead of as a native string that is
  then copied and interned: pydantic's import runs about 0.7M fewer instructions (the
  literals' share 1.75M -> 1.10M), mistune's 0.15M. The sites of one text share one
  object, as CPython's merged constants of a module do: two equal literals that are not
  interned (`'a b'`) are now the same object, as in CPython.
- **CPython 3.15 is the oracle** (was 3.13): CI, the fuzzer, the benchmarks and the
  release builds run CPython 3.15 (3.15.1 on the hosted runners), and native programs match 3.15's behavior: every
  `ZeroDivisionError` says `division by zero`, `0.0 ** -1` says `zero to a negative
  power`, `math` domain errors name the expected input (`expected a nonnegative input,
  got -1.0`), an unpacking of too many values says how many (`expected 2, got 3`),
  `list.index`/`deque.index`/`deque.remove` say `x not in list`/`x not in deque`, a
  slice index error drops `or None`, `%c` out of range says `format argument: %c
  argument not in range(0x110000)`, a set display of constants iterates as the set built
  once in source order, and `str` predicates use Unicode 17.0.0's tables.
- Supported versions: `--ext` modules and `pycc.build` wheels keep the `cp313-abi3`
  floor and import on CPython 3.13, 3.14 and 3.15. The fast paths that read CPython's
  internal layouts (compiled functions in real CPython frames, the attribute and
  module-global caches) now know 3.15's
  layouts as well as 3.13's; on 3.15 a module global rebound from Python reaches
  compiled code through one dict watcher shared by every pycc module, as 3.14 removed
  the dict version tag. A module imported by 3.14 runs without those fast paths, as
  before. CI no longer runs the test suite against 3.13.
- On Windows a native program's standard streams follow CPython 3.15's UTF-8 mode
  default (PEP 686): UTF-8 unless `PYTHONUTF8=0` (or `PYTHONIOENCODING`) says otherwise,
  where 3.13 used the ANSI code page.
- Compiled code of a non-strict `--ext` module reading a global that only fallback code
  binds reads it as `LOAD_GLOBAL` does: a name the module never binds is the builtin's
  (tomli binds `frozendict` only before 3.15, where it is a builtin), and on 3.15 a lazy
  import (`__lazy_modules__`) is resolved and replaced in the namespace.
- An error message the extension shim raises that CPython words differently per version
  follows the interpreter hosting the module (`Value after ** must be a mapping` on 3.15).
- A Linux `--ext` module reports a runtime panic or allocation failure in one line and
  aborts, as before, through the runtime's own handlers (`crates/pycc_rt/src/abort.rs`):
  the link wraps the standard library's two handlers to them, so its stack symbolizer
  (`gimli`, `addr2line`, `miniz_oxide`, `rustc_demangle`) is no longer linked in. No
  backtrace is printed, even with `RUST_BACKTRACE` (`docs/DEVIATIONS.md`). pystache's
  extension loses 199 KB of its 971 KB of code and 13 of its 362 symbols bound at
  `dlopen`; its import measured 12.11 -> 11.95 ms (CPython 11.22 ms), pydantic's
  77.6 -> 76.7 ms (CPython 81.9 ms).
- The pydantic benchmark builds pydantic 2.14.2 with pydantic-core 2.50.2 (2.13.5's
  pydantic-core has no CPython 3.15 wheels); the pinned numpy, Cython and mypy wheels are
  their cp315 builds.

### Fixed
- `sorted()` and `list.sort()` (with or without `key=`/`reverse=`) are CPython's own
  timsort, run detection, binary insertion, powersort merges and galloping included, so a
  `nan` among floats ends where CPython leaves it (`sorted([2.675, 0, nan])` is
  `[0, nan, 2.675]`). Before, a merge sort put it elsewhere (found by the fuzzer).
- The differential fuzzer no longer compares the warnings CPython writes to stderr: a
  `DeprecationWarning` (`~` on a `bool`) printed before an uncaught `SystemExit`'s message,
  which has no traceback to separate them, was reported as a different exception (nightly
  runs since 2026-11-04). `docs/DEVIATIONS.md` documents that compiled `~` on a `bool`
  does not warn.
- A compiled body's values that an exception unwinds (the items of a list or tuple display
  before the one that raised, a loop's iterator) are released with the exception set
  aside, as CPython's unwinding drops them. A `__del__` they run, compiled or not, no
  longer sees the pending exception as its own failure. Before, it was reported as
  ignored and the original exception became a `SystemError`.
- A non-strict `--ext` build no longer fails on a selected function whose comprehension
  holds a walrus (`[y for x in xs if (y := f(x))]`, in pydantic's `_fields.py` and
  `networks.py`): the walrus target is the function's own local (PEP 572), not a module
  global to read through the namespace, which the build had rewritten into a subscript
  and then failed to parse. The function stays bytecode (`C0001`).
- A module-level function of a non-strict `--ext` module used as a value that crosses
  into CPython (in a dict or tuple display, returned) is the function the module binds
  under its name, as CPython code reads it, not a positional-only wrapper: calls with
  keywords and defaults work (`functools.partial(f, limit=3)`). pydantic's constraint
  validators took this path; its test suite now fails only what the sources fail, plus
  `test_type_checking_model_dump_signature_in_sync_with_base_model`, which loads
  `root_model.__file__` (a package build's documented `__file__`).
- A function with a default (or `*args`) and a parameter named as the function itself
  (`def f(x, f=None)`) called its argument instead of itself from its `--ext` signature
  entry; the entry now names that parameter apart (`_pycc_s_f`).
- A published compiled class of an `--ext` module has CPython's `__doc__` (its body's
  docstring, cleaned as the compiler cleans it), `__firstlineno__` and
  `__static_attributes__`; it had `__doc__ = None` and neither of the others. A class
  body's own `__doc__ = "..."` or `__firstlineno__ = N` wins, as in CPython, and is not
  copied to a compiled subclass; such a `__doc__` on a class made from its template
  (a `Generic[T]` base) made the import fail with `TypeError: cannot set '__doc__'
  attribute of immutable type`. A docstring of a CRLF source is cleaned as CPython
  cleans it (its line breaks read as `\n` first).

## [0.11.0] - 2026-11-01

Imports faster than CPython -- accepts 2 (smaller, faster-loading extensions) and 3
(less work at import) are met, and accept 4 for mistune, whose import is now faster than
CPython's; accept 1 (pydantic's import >= 1.0x) and accept 4 for pystache are **not
met**. pydantic's import went from 0.89x / 0.93x of CPython (x86_64 / aarch64, the
2026-10-30 baseline) to **0.97x / 0.98x**; the remaining gap is spread thin (details in
[the v0.11 roadmap](https://github.com/rotnov/pycc-next/blob/main/docs/roadmap/v0.11.md)).

### Added
- `bench/package_check.py --time` times `import mistune` and `import pystache`, one import
  per fresh process against the CPython tree's `.pyc` files, so the CI benchmark job
  publishes both (v0.11 accept 4); a workload CPython runs in under 0.1 s prints and
  publishes in ms.

### Changed
- A fallback chunk of only `def` statements whose defaults, keyword defaults and
  annotations are constants (no decorator, no closure), and a nested statement's scaffold,
  is made at import by the module's `__pycc_run__` hook from the functions' own code
  objects, without the chunk's code object or an `exec` frame (v0.11 accept 3); every
  other chunk runs as `exec` of its code, as before. 105 of the CI workload's 443 chunks
  (3.13 only; `-O` still compiles the source). The rewrite calls `ns["__pycc_run__"](k)`
  where it called `builtins.exec(ns["__pycc_code__"](k), ns)`; an audit hook sees a
  `function.__new__` event per function made this way (`docs/DEVIATIONS.md`). Own import share:
  pystache 8.27M -> 8.23M, mistune 23.60M -> 23.52M, pydantic 91.09M -> 90.63M (the CI
  workload's import phase 213.25M -> 212.67M).
- A compiled import statement whose names fallback code reads is mirrored into the
  module's namespace by compiled code storing what its own import bound, instead of
  running the statement again as a chunk (v0.11 accept 3): the statement calls
  `__import__` once, where it called it twice, and each name binds the value its own
  import took. Compiled code's imports pass `__import__` the module's namespace as
  `globals` (and as `locals` in its body), as CPython's `IMPORT_NAME` does, where they
  passed None. Own import share: pystache 8.66M -> 8.27M, mistune 23.73M -> 23.60M,
  pydantic 92.61M -> 91.09M (the CI workload's import phase 215.11M -> 213.25M).
- A compiled module's forwarder slots no longer copy each function's parameter names and
  positional defaults (24 of each, 488 bytes a slot, zeroed for every slot when the
  module's body started); a call that binds keywords or defaults reads them from the
  function's own tuples (v0.11 accept 3). Own import share: pystache 8.68M -> 8.66M,
  mistune 23.80M -> 23.73M, pydantic 92.82M -> 92.61M (the CI workload's import phase
  215.39M -> 215.11M).
- The code objects a compiled module's import used to re-qualify and flag with
  `code.replace` -- each nested `def` CPython runs (its scaffold's `__qualname__`) and each
  published signature's wrapper -- are rewritten when the build compiles their chunks, so
  an import from the pre-compiled table makes none of those calls (about 13K instructions
  each, 110 in the CI benchmark's pydantic import); under `-O`, where the embedded source
  is compiled at import, the source still makes them (v0.11 accept 3). Own import share:
  pystache 8.83M -> 8.68M, mistune 24.06M -> 23.80M, pydantic 93.69M -> 92.82M (the CI
  workload's import phase 216.69M -> 215.39M).
- A compiled module's import looks names up by interned `str` keys instead of C strings
  (v0.11 accept 3): a package member's interface table is filled by the keys its body
  published its functions with, the fallback chunks' interpreter check (magic number,
  `-O`, `marshal.loads`) runs once per extension instead of once per module, and the
  shim's fixed-name lookups (`__file__`, `__builtins__`, `__dict__`, `flush`, the package
  finder's `__spec__`/`name`) use interned names. Own import share: pystache 9.0M ->
  8.8M, mistune 24.4M -> 24.1M, pydantic 94.5M -> 93.7M. Lever 8 (one code object per
  module's fallback chunks) was measured and rejected: about -0.4M net
  (`docs/roadmap/v0.11.md`).
- ELF extension modules built for a glibc that reads `DT_RELR` (2.36 and newer; not with
  `--target`), with a linker that packs them (a probe link checks for `DT_RELR`; GNU ld
  2.42 does on x86-64, not on aarch64), link with `-z pack-relative-relocs`: pydantic's
  whole-package extension is 4.6% smaller (10.75 MB -> 10.26 MB) and has 349 relocation
  entries instead of 21,222 (mistune's: -5.5%, 9,395 -> 348). An extension's module body
  is placed with the import-only code (`cold`). No CI instruction bound moves; LTO
  across a package's modules, identical-code folding and hot/cold splitting were measured
  and do not pay (v0.11 accept 2, `docs/roadmap/v0.11.md`).
- On Linux every compiled function goes in an object section of its own, as with
  `-ffunction-sections`: the assembler laid out one `.text` section holding a whole large
  module again and again until every branch had its size (17% of all the instructions
  building diff-match-patch). The linker merges the sections back into `.text`. Building
  diff-match-patch takes 8.6 s instead of 9.1 s locally (median of 5 interleaved builds).
- Importing a compiled module no longer compiles source with `compile()` (v0.11 accept
  3): published classes are made by a class maker written in C, their statement's
  arguments read from a build-time plan when they are bare names, and the compiled-frames
  probe builds its template code object with `types.CodeType`. pystache's own import
  share drops from 15.7M to 10.5M instructions, mistune's from 28.9M to 25.9M, pydantic's
  from 102.4M to 99.7M (CPython: 7.6M, 29.0M, 88.5M). The import also no longer adds a
  `__pycc_frame_template__` attribute to `sys`.
- A compiled frame's code object costs a third of what it did to make (v0.11 accept 3):
  its `co_filename` is joined in C on POSIX instead of through `os.path`, and it is made
  by `types.CodeType` with positional arguments instead of `code.replace` with keywords.
  A compiled module's `__builtins__` is the builtins' dict, as `exec` stores it in a
  Python module's namespace, instead of the `builtins` module. Own import share: pystache
  10.5M -> 10.1M, mistune 25.9M -> 25.1M, pydantic 99.7M -> 98.5M.
- A compiled `from m import a, b` statement calls `__import__` once, as CPython does,
  instead of once per name (v0.11 accept 3); a hook on `builtins.__import__` sees the same
  calls for the compiled module as for its source.
- Patching the frameless forwarders of a class CPython creates costs less than half what it
  did (v0.11 accept 3): a forwarder's code is read at CPython 3.13's offsets, checked once
  against its attributes, and one class's methods resolve the class once. Own import
  share: pystache 9.2M -> 9.1M, mistune 25.0M -> 24.6M, pydantic 97.5M -> 95.8M.
- A string literal's first evaluation interns it without copying its bytes or hashing them
  with SipHash (v0.11 accept 3). Own import share: mistune 24.6M -> 24.4M, pydantic 95.8M
  -> 95.2M.
- Publishing a compiled module's functions as its body defines them costs one pass over
  its function table instead of one pass per function (v0.11 accept 3): the lookup starts
  after the previous function it found. Own import share: pydantic 95.2M -> 94.6M.

### Fixed
- A `return` from inside a `for x in <object>:` loop releases the loop's iterator, as
  CPython does, including through `finally` clauses inside and around the loop. Each such
  return used to leak one iterator: after mistune's test suite the compiled tree held 28,328
  `list_iterator` objects that CPython does not (#42).
- Importing a compiled package no longer imports `importlib`, `importlib._bootstrap`,
  `importlib._bootstrap_external` and `importlib.machinery`: its finder takes `ModuleSpec`
  from the frozen import system. Own import share with the change above: pystache 10.1M ->
  9.2M, mistune 25.1M -> 25.0M, pydantic 98.5M -> 97.5M.
- `--non-strict` (native build and `--ext`): fallback code reaching module globals by a
  string -- `eval("c")`, `exec(src)`, `globals()["c"]`, `locals()`/`vars()`/`dir()` at
  module level, or the module object (`sys.modules[__name__]`, `import __main__`) -- now
  sees every compiled global, a compiled class's instance included, where it raised
  `NameError` for any global no statement of it names (issue #20). A variable such code
  binds lives in the module namespace, so compiled code reads back what it stores.
- `--non-strict`: a module-level type variable (`T = TypeVar("T")`) whose `TypeVar`
  import runs in CPython was made twice, by compiled code and by the CPython copy of the
  assignment, and compiled code held the second object; it runs in CPython alone now, so
  the import makes one and compiled code reads it (importing pydantic made 39 type
  variables against CPython's 22; 22 now, and 0.4M fewer instructions).

### Known gaps
- v0.11 accept 1 (not met): pydantic 2.13.5's import is **0.97x** CPython's on x86_64
  (90.78 ms against 87.81 ms) and **0.98x** on aarch64 (72.13 ms against 70.89 ms), median
  of 15 runs (bench run 41248409468). In instructions the CI workload's import phase costs
  212.67M against CPython's 208.77M (callgrind, PBS 3.13.13); at the last breakdown no
  item of the remaining gap was above 0.75M (unmarshalling the fallback chunk tables, the
  `str` literal cache, string hashing, relocations), and decorated `def`s and class
  statements still run as `exec`'d chunks so that their tracebacks stay exact. Moved to
  v0.12.
- v0.11 accept 4 (not met for pystache): pystache 0.6.8's import is 0.97x CPython's on
  x86_64 (11.51 ms against 11.18 ms) and 0.94x on aarch64 (10.02 ms against 9.40 ms).
  Moved to v0.12.
- v0.10 accept 3 is still not met: pystache's own benchmark runs at 0.64x CPython and
  mistune's `html` workload at 0.86x (x86_64, same run).
- After `import pydantic`, 33 `__pycc_lift*` names remain in its modules' dicts (lever 6
  of the v0.11 plan, not done).
- Open rows of `docs/DEVIATIONS.md`.

### Speed
Bench run [41248409468](https://github.com/rotnov/pycc-next/actions/runs/41248409468) at
6ab165e (ubuntu-24.04, AMD EPYC 7763; ubuntu-24.04-arm, Neoverse-N2; medians of 11
interleaved fresh-process runs, pydantic of 15). Ratios are CPython's median over
pycc-next's; above 1, pycc-next is faster.

| workload | CPython | pycc-next | vs CPython | vs Cython 3.3.0 | vs mypyc |
|---|---|---|---|---|---|
| nbody | 3.176s | 0.072s | 44.0x | 74.35x faster | 39.26x faster |
| spectral_norm | 1.467s | 0.017s | 87.9x | 91.35x faster | 5.76x faster |
| diff-match-patch `diff_main` | 14.245s | 0.975s | 14.61x | 11.90x faster | does not compile |
| pypng read, five filters | 0.140s | 0.008s | 17.46x | 11.38x faster | does not compile |
| tomli `loads` x100 | 68.70 ms | 38.90 ms | 1.77x | 1.32x faster | 1.40x slower |
| pystache benchmark x1000 | 81.80 ms | 128.40 ms | 0.64x | 1.91x slower | does not compile |
| mistune `html` x20 | 0.176s | 0.205s | 0.86x | suite fails | does not compile |
| pydantic define | 9.96 ms | 12.27 ms | 0.81x | 1.18x slower | does not compile |
| pydantic schema | 22.26 ms | 26.79 ms | 0.83x | 1.36x slower | does not compile |
| pydantic roundtrip | 22.58 ms | 24.71 ms | 0.91x | 1.07x slower | does not compile |

Imports, both runners (one import per fresh process; pydantic: `bench/pydantic_workload.py`'s
import phase):

| import | x86_64 CPython | x86_64 pycc-next | ratio | aarch64 CPython | aarch64 pycc-next | ratio |
|---|---|---|---|---|---|---|
| pydantic | 87.81 ms | 90.78 ms | 0.97x | 70.89 ms | 72.13 ms | 0.98x |
| mistune | 33.09 ms | 31.95 ms | **1.04x** | 28.29 ms | 27.38 ms | **1.03x** |
| pystache | 11.18 ms | 11.51 ms | 0.97x | 9.40 ms | 10.02 ms | 0.94x |

aarch64's pydantic define, schema and roundtrip phases: 0.78x, 0.82x, 0.92x. The run's job
summary has every program, package workload and phase, with min-max.

Earlier versions: [0.10](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.10.md), [0.9](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.9.md), [0.8](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.8.md),
[0.6-0.7](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.6-0.7.md), [0.4-0.5](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.4-0.5.md),
[0.1-0.3](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.1-0.3.md).
