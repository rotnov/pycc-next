# Changelog

All notable changes to pycc-next. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Changed
- An `--ext` module's import-time tables of names and sources (its interface names,
  published class names, builtin exception names, precompiled fallback chunks and
  their file names) are one string each indexed by offset, and its forward
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
- An `--ext` module's precompiled fallback chunk holding `frozenset` constants
  rebuilds each in source order along the path the build recorded to it, instead of
  walking every nested code object at import: pydantic's import runs 1.55M fewer
  instructions (of about 143M). The constants iterate as before.
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
  module-global caches, `def`s made directly from precompiled bytecode) now know 3.15's
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
- The pydantic benchmark builds pydantic 2.14.2 with pydantic-core 2.50.2 (2.13.5's
  pydantic-core has no CPython 3.15 wheels); the pinned numpy, Cython and mypy wheels are
  their cp315 builds.

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
