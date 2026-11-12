# Changelog

All notable changes to pycc-next. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Fixed

- A profile-selected function's compiled body is specialized for a parameter's exact type
  when that type covers at least 95% of the calls pass 2 sampled, not only when it covers
  all of them (`profile::DOMINANT_PERCENT`, `docs/design/compile-little.md` section 2.5).
  pypng's test suite passes `undo_filter_*` an `array('B')` 1-2 times in 1,000 calls, which
  left the filter loops untyped: from the same profile, pypng's five-filter read goes from
  1.74x to 17.85x CPython's instructions (1.59x to 10.1x wall time) and its PngSuite
  decode from 1.23x to 3.55x (1.11x to 2.61x); tomli is unchanged (1.46x instructions).
  An argument of another type fails the exact-type guard and runs the generic compiled
  body, as before, so results do not change.

- `pycc --help` and `pycc <command> --help` describe every subcommand and flag for the
  user. v0.12.0 printed internal doc comments there (`run`, `check` and `lock` showed
  "Non-strict mode (#3)...", `build` a design-document reference, flag help carried issue
  numbers and decision ids) and no description at all for `test`, `explain`, `init`,
  `clean` and `version`. `tests/cli_help.rs` checks every subcommand's description and help.

## [0.12.0] - 2026-11-12

Compile little, compile well -- non-strict `--ext` builds now compile only the functions a
`pycc profile` run measures as winners, and the modules run as CPython bytecode. pydantic's
`model_json_schema()` is **1.20x / 1.23x** CPython's speed (x86_64 / aarch64; v0.11 0.83x /
0.82x against CPython 3.13), model definition 1.00x / 1.01x and the whole process 1.07x /
1.07x, so v0.12 accept 3's pydantic targets are met but one, and pystache's import is at
parity (1.01x / 1.00x, accept 2). pydantic's import is **not** above CPython's (0.99x /
0.99x: accept 1 and accept 3's "import stays >= 1.0x" are not met). pypng and tomli, built
whole-module in v0.11, are slower than they were (see Known gaps). Details in
[the v0.12 roadmap](https://github.com/rotnov/pycc-next/blob/main/docs/roadmap/v0.12.md).

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
- `pycc profile`'s pass 3 prunes its winning set (`docs/design/compile-little.md`,
  section 2.3): each member, least self time first, is removed in turn and the set
  without it measured as a whole against the winner, then the members whose removal did
  better at all are removed together and that set measured once more. The selection
  drops them only when the set without them beats the winner as a larger set must beat a
  smaller one (counted: 0.5% of the workload's instructions fewer, with the wall-time
  veto; on wall time: apart beyond the noise and 5% of CPython's median faster); else a
  single removal that does; else the winner stays whole. At most 8 builds more
  (`MAX_PRUNE_BUILDS`: 7 removals and the group); pass 3 prints each removal and which
  members were dropped, and a dropped function's rejection names both sets'
  instructions. On pydantic's training workload no single removal saves 0.5%, but
  `GenerateJsonSchema.mode`, `GenerateJsonSchema._config` and `create_schema_validator`
  together save 0.88%: the selection goes from 9 functions at -4.31% of the
  instructions to 6 at -5.19% (pass 3: 11 builds, 296 s).
- A profile's observed type specializes a parameter only where a loop of the function
  computes with its native value (`docs/design/call-site-specialization.md`): an
  operation whose other operand is an object converts on every iteration, which the
  generic body, with its inline exact-type paths, never does. tomli's ten observed-type
  specializations each made the workload run more instructions (`skip_chars`, whose
  `src: str` meets only the object `pos` and an object frozenset, 1,325 more per call),
  so pass 3 kept none of its functions; none is specialized now, and pass 3 keeps its 19
  hot functions at 14.5% fewer instructions than CPython. `--report-fallbacks` names each
  parameter so left an object (`meets no native operand in a loop`).
- `pycc profile`'s pass 3 decides its checks on the training workload's instruction
  counts under callgrind when it can (`--verify-signal auto|instructions|wall`, replacing
  `--verify-instructions`; `docs/design/compile-little.md`, section 2.3): fresh-process
  wall times of a package's test suite vary by about 15% from run to run, which made a
  set saving a few percent a coin toss (pypng's selection flipped between 8 and 30
  functions, diff-match-patch's between 1 and 30), while the counts repeat to a few
  instructions in billions. A set wins at 0.5% fewer instructions and no slower beyond
  the noise of its wall times, and a larger set replaces a smaller one only by saving
  0.5% more. `auto` counts when valgrind runs the workload and CPython runs it in at
  most a second; else wall time decides as before, where a larger set must now also save
  5% of CPython's median time more. `--verify-rounds` is now the most rounds a check runs (default
  15; it stops after 5 once the wall times are apart).
- Non-strict generic bodies (`docs/design/compile-little.md`, sections 2.6 and 2.7;
  `PYCC_NO_FAST_OPS=1` turns each off):
  - `try: d[k]` with a first handler that catches `KeyError` and neither binds the
    exception nor re-raises it: a key missing from an exact `dict` goes straight to the
    handler, with no exception made; the handler keeps the key until it is left. Other
    mappings and other failures raise as before. The `KeyError` is made the first time
    something in the handler could see it (see Fixed, below), so a handler that returns a
    default, or finds the key in a second dict of `str` keys, makes none.
  - `tuple(a)`, `list(a)` and `sorted(a)` of an object no longer go through a call of the
    builtin when it is CPython's own.
  - A comprehension that unpacks its item (`for k, v in d.items()`) over a dict's own
    `items()` releases each item before it advances, so the iterator reuses its pair
    tuple.
  With pydantic's three hottest schema functions compiled, the schema phase measures
  about 1.12x CPython locally (1.07x before); `ConfigWrapper.__getattr__` replays at
  14.2M instructions against 46.3M before.
- More idiom lowerings in non-strict generic bodies (`docs/design/compile-little.md`,
  section 2.6; `PYCC_NO_FAST_OPS=1` turns each off):
  - A comprehension that unpacks its item into two names (`[f(k, v) for k, v in src]`)
    binds them straight from the source. Over `d.items()` of an exact `dict` each step is
    `PyDict_Next`, with no view, iterator or pair tuple, and the dict iterator's
    `RuntimeError`s when the dict changes size or keys meanwhile; any other source is
    iterated and each item unpacked as CPython unpacks it, so an item that is iterable but
    not subscriptable (an iterator, a set) now unpacks, and one of the wrong length raises
    CPython's `ValueError`. This replaces the early release of a dict's pair tuples.
  - `sorted()` of a generator expression sorts the list the generator was materialized
    into in place, and `tuple()` of a list nothing else holds moves its items into the
    tuple.
  - A tuple display with an object element is built by `PyTuple_New` with its items
    stored in place.
  pydantic's `_make_json_hashable` replays at 86.1M instructions (120.9M before; the
  hand-written C, 53.3M) and `_deduplicate_schemas` at 97.3M (132.1M); with the three
  section 1 functions compiled, the schema phase runs 425.2M instructions against
  CPython's 558.4M (460.1M before) and measures about 1.18x CPython locally (1.14x before).
- Generic bodies keep a scanner's `int` position and current one-character `str` as
  machine words (`docs/design/call-site-specialization.md`, "Tagged locals"): an `object` local
  rebound as `x = x ± y` or `c = s[i]` and read as a word (an index into an exact
  sequence, a comparison with a one-character or `int` literal, `in`, a `str` slice
  bound) holds a tagged small `int` or ASCII character, boxed only where another read
  needs the object. `s[a:b]` and the
  truth of `s.startswith(t, a)` on an exact `str` with small-`int` operands no longer
  build a `slice`, box the bounds or call a bound method. A direct call checks the
  recursion limit once per invocation of the calling body on CPython 3.15. Local
  callgrind, `--select all`: tomli `loads()` of `Cargo.lock` 22.92M -> 20.16M
  instructions; package checks diff-match-patch 158.84M -> 157.28M, tomli 2.95M ->
  2.77M, pypng 147.20M -> 142.94M, mistune 59.25M -> 58.65M, pystache 0.45M -> 0.44M;
  pydantic's `schema` phase 513.4M -> 510.5M.
- `pycc profile`'s pass 3 selects a larger winning set over a smaller one only when it
  is faster beyond the noise of both (its compiled third quartile under the smaller one's
  first quartile); before, the lower median won, so two winners with overlapping
  interquartile ranges flipped from run to run (diff-match-patch: a 30-function region or
  `diff_bisect` alone).
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
- CI's compile-speed gate (v0.5 accept 4) no longer compares seconds of wall time with
  10 s, which flipped with the runner's CPU model: `bench/package_check.py --select all
  --build-ratio` fails a package whose build's CPU time is over its bound
  (`max_build_ratio`, 15% above CI's median) times that of a fixed `clang -O2` compile
  run first (`bench/build_clock.py`); `--max-build-seconds` is gone. The derivation is in
  `docs/roadmap/v0.1-v0.7.md`.

### Fixed
- `pycc profile`'s pass 3 could stall forever on one callgrind run: on GitHub's arm64
  runner, valgrind ran pypng's training workload past 45 minutes. Every callgrind run now
  has a deadline (`docs/design/compile-little.md`, section 2.3): CPython's run 300 times
  CPython's median wall time, each check's 4 times what CPython's callgrind run took,
  never under 120 s. Past it the run is killed (on Linux with every process it started) and fails
  with "callgrind did not finish within N s": under `--verify-signal auto` the pass (or
  that check) then decides on wall time; under `instructions` it is an error. The
  probe's 3 CPython runs, which set the deadline, now run under `instructions` too.
- `bench/pydantic_check.py` compiled nothing: its training profile ran without the
  build's `--exclude v1`, so pass 3's winning set was not valid for the build, and the
  CI benchmark timed CPython against CPython. The profile now names the build's excludes,
  the build targets the interpreter the profile ran (`PYCC_PYTHON`, unless set), and the
  script imports again (its `Package` lacked `max_build_ratio`). The measured import
  breakdown is in `docs/ROADMAP.md` (v0.12, accept 1).
- Non-strict `--ext`: an object freed while an exception propagates through compiled code
  (a local of the frame it leaves, a missing key the handler held, the argument of a
  `sorted()` that failed) may run compiled code of its own, a `__del__` or a generator's
  `finally`. That code read the pending exception as its own failure: CPython printed
  `Exception ignored in ...`, the exception was lost, the `except` clause it was
  propagating to did not run, and the function returned `None` or raised `SystemError:
  object result was NULL`. While an exception is pending, an object is now freed with it
  set aside, wherever the release is (`pycc_ext_obj_dealloc`); a release on any other
  path executes what it executed before.
- An exception that is its own cause (`except ValueError as e: raise e from e`) was never
  freed, nor the CPython exception it stands for: one per call. It no longer holds a
  reference to itself.
- `--ext --non-strict`: `raise o from cause` with `o` a CPython object and the cause a
  compiled exception (`raise o from KeyError("c")`, `except KeyError as k: raise o from k`)
  never freed the cause: one per call. The cause is now released after the raise, and an
  exception's own record of its cause is released once its CPython object holds it.
- `--ext`: a compiled `except` clause publishes the exception it handles, as CPython's
  does. Inside it `sys.exception()` and `sys.exc_info()` returned `None` and
  `traceback.format_exc()` was empty, and an exception raised there had no `__context__`;
  they now see the handled exception, which becomes the context of what the clause raises
  (`raise X from None` marks it suppressed and drops an earlier cause, `raise X from Y`
  sets `__cause__`, neither of which compiled code did; a cause never raised keeps no
  `__context__`; a handler that catches `e` again sees the cause the last `raise e from`
  gave it, and an edit Python code makes to `__cause__` or `__suppress_context__` stays:
  once an exception has a CPython object, that object alone holds those links), and it is
  the previous one again once the clause is left. Raising a
  stored exception (`raise e`) gives it the handled one as its context, cutting a cycle as
  CPython does, and so does a `KeyError` for a non-`str` key, which had none. The
  clause publishes the first time something could see the exception (a call, a `raise`, a
  release that frees an object), so entering a handler costs what it did and the path
  without an exception nothing. A `finally` clause, an `except*` clause and a generator's
  handlers do not publish yet (`docs/DEVIATIONS.md`).
- Non-strict `--ext`: a comprehension over an object whose element or condition raised
  kept its current item until the process ended (one reference leaked per failing call):
  the item is released on that path too, after the iterator and the result, and a
  comprehension that ends releases its exhausted iterator before its last item, both as
  CPython does.
- Non-strict: `"..." % e` with a caught exception `e` (alone or in a tuple of arguments)
  read the exception as another kind of object: it printed garbage (`caught _io`) or
  crashed. Only an operand codegen can box is formatted in compiled code; a function that
  formats an exception or an enum member this way runs in CPython (`I0404` in
  `--report-fallbacks`).
- An exception no `except*` clause matched propagates as the same object, with its
  traceback entries, and the subgroups an `except*` statement derives carry the original's
  entries on; the remainder was a new group that had lost them.
- A compiled frame's traceback no longer gains a second entry, at the line of the last
  statement of the `try` statement, when an exception passes that statement: an unmatched
  handler, a bare `raise` and a `finally` clause add none, as CPython's `RERAISE` adds
  none.
- `abs(-2**62)`, the most negative smallint, is `2**62` where the argument's range is known
  at compile time (a literal, a local, a `range()` loop variable); it printed
  `-4611686018427387904` (found by the fuzzer).
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

### Known gaps
- v0.12 accept 1 and accept 3's import target (not met): pydantic 2.14.2's import is
  **0.99x** CPython's on x86_64 (93.76 ms against 92.94 ms) and **0.99x** on aarch64
  (74.40 ms against 73.91 ms), median of 15 runs (bench run 43192816730), inside the spread
  on both; under callgrind its import phase runs 470.16M instructions against 469.82M. The
  modules run as bytecode, so the import is at parity by construction; what can make it
  faster is building the code objects for the texts CPython compiles at import
  (`dataclasses` methods, forward references), prototyped at 1.14x. Moved to v0.13.
- Profile builds of pypng and tomli are slower than v0.11's whole-module builds:
  pypng's five-filter read runs 2.02x / 1.95x CPython (10.42x / 17.20x in bench run
  41750538464 at 0f97b93, before the switch), its PngSuite read 1.24x / 1.20x (3.10x /
  3.69x), tomli's `loads` 1.42x / 1.31x (1.70x / 1.49x). Moved to v0.13.
- v0.10 accept 3 is still not met: pystache's own benchmark and mistune's `html` workload
  run at CPython's speed (1.00x and 0.99x on x86_64), not 1.2x faster.
- Open rows of `docs/DEVIATIONS.md`.

### Speed
Bench run [43192816730](https://github.com/rotnov/pycc-next/actions/runs/43192816730) at
5b7cc6c (CPython 3.15.2; ubuntu-24.04, AMD EPYC 7763; ubuntu-24.04-arm, Neoverse-N2; 4
vCPUs each; medians of 11 interleaved fresh-process runs, pydantic of 15). Ratios are
CPython's median over pycc-next's; above 1, pycc-next is faster. Cython's mode is its
default (annotations honored) except tomli and pydantic, where that build fails tomli's
suite or does not compile, so their Cython numbers ignore annotations.

| workload | CPython | pycc-next | vs CPython | vs Cython 3.3.0 | vs mypyc |
|---|---|---|---|---|---|
| nbody | 3.888s | 0.072s | 54.0x | 79.31x faster | 41.47x faster |
| spectral_norm | 1.229s | 0.017s | 74.0x | 81.74x faster | 5.90x faster |
| diff-match-patch `diff_main` | 10.368s | 0.692s | 14.98x | 13.66x faster | does not compile |
| pypng read, five filters | 0.108s | 0.054s | 2.02x | 1.21x faster | does not compile |
| pypng read, PngSuite | 79.70 ms | 64.10 ms | 1.24x | **1.17x slower** | does not compile |
| tomli `loads` x100 | 66.50 ms | 46.90 ms | 1.42x | 1.08x faster | **1.71x slower** |
| pystache benchmark x1000 | 77.20 ms | 77.20 ms | 1.00x | **1.13x slower** | does not compile |
| mistune `html` x20 | 0.168s | 0.170s | 0.99x | suite fails | does not compile |
| pydantic define | 8.89 ms | 8.87 ms | 1.00x | 1.05x faster | does not compile |
| pydantic schema | 21.88 ms | 18.25 ms | **1.20x** | 1.03x faster | does not compile |
| pydantic roundtrip | 25.42 ms | 24.89 ms | 1.02x | 1.04x faster | does not compile |
| pydantic whole process | 1274.95 ms | 1195.85 ms | 1.07x | 1.04x faster | does not compile |

Imports, both runners (one import per fresh process; pydantic: `bench/pydantic_workload.py`'s
import phase):

| import | x86_64 CPython | x86_64 pycc-next | ratio | aarch64 CPython | aarch64 pycc-next | ratio |
|---|---|---|---|---|---|---|
| pydantic | 92.94 ms | 93.76 ms | 0.99x | 73.91 ms | 74.40 ms | 0.99x |
| mistune | 25.99 ms | 26.13 ms | 0.99x | 21.09 ms | 21.06 ms | 1.00x |
| pystache | 11.90 ms | 11.83 ms | 1.01x | 10.07 ms | 10.10 ms | 1.00x |

aarch64's pydantic define, schema, roundtrip and whole process: 1.01x, **1.23x**, 1.01x,
1.07x (the profile build compiles 5 of pydantic's functions on x86_64 and 3 on aarch64);
diff-match-patch 16.18x, pypng 1.95x and 1.20x, tomli 1.31x, pystache's benchmark 1.00x,
mistune's `html` 1.00x. The run's job summary has every program, package workload and
phase, with min-max, and pydantic's callgrind counts.

Earlier versions: [0.11](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.11.md), [0.10](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.10.md), [0.9](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.9.md), [0.8](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.8.md),
[0.6-0.7](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.6-0.7.md), [0.4-0.5](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.4-0.5.md),
[0.1-0.3](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.1-0.3.md).
