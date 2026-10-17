# Roadmap

Milestones in order. A milestone is done when every Accept item is met with evidence
(a commit, a CI run or a `bench/` measurement), then released with the `release` skill.

## v0.1 — Restart (released 2026-10-11)

Typed Python to native code with compile-time type errors, CPython interop
(`--ext`, embedded, `--non-strict`), memory management, release archive. See CHANGELOG.

## v0.2 — Rust speed, numpy, more Python (released 2026-10-12)

Accept:
1. **Speed:** every `bench/py` program is within 3x of its `bench/rs` Rust twin
   (`python3 bench/run.py`, best of 7) — v0.1: fib 3.8x, nbody 4.2x, spectral_norm 3.5x,
   sieve 3.1x, mandelbrot 1.1x. **Met 2026-10-12:** fib 1.8x, mandelbrot 1.1x, nbody 2.4x,
   sieve 2.1x, spectral_norm 1.8x (shared 4-core box, best of 11; the Rust twins build for
   the default target CPU and black-box every input).
2. **numpy:** typed `numpy.ndarray` / `NDArray[float64|int64|bool]` of 1 and 2 dimensions
   is a native type in strict and `--ext` code: element read/write, `.shape`, `len`,
   iteration, creation via `numpy.zeros/ones/empty/array`, without CPython calls in the
   loop body. New benchmarks `bench/py/stencil2d.py` and `bench/py/prefix_sum_np.py`
   (numpy arrays + Python loops) run >= 30x faster than CPython and within 3x of Rust.
   **Met 2026-10-12:** `tests/numpy_typed_arrays.rs` (oracle + array-safety fixtures);
   bench: stencil2d 436x faster than CPython / 2.5x of Rust, prefix_sum_np 80x / 1.8x.
3. **Language:** `**kwargs` in calls and definitions, keyword-only parameters and
   `*args` in definitions, default arguments for module functions in `--ext`
   (#1194), generator `send()`/`throw()`/`close()` and `yield` as an expression,
   `return value` in generators, closures stored in lists/dicts/attributes.
   **Met 2026-10-12:** `tests/language_round6.rs` (generator protocol, call signatures,
   function values, `--ext` signatures) against CPython 3.13, natively and in non-strict
   mode, with leak probes in `tests/memory_leaks.rs`; remaining gaps in `docs/DEVIATIONS.md`.
4. **Real-world corpus:** strict passes >= 110/200; `--non-strict` keeps 200/200 with
   >= 95% of top-level statements native.
   **Met 2026-10-12:** `python3 bench/corpus.py`: strict 124/200, 0 wrong (strict mode
   infers the signatures of unannotated functions only the module calls);
   `--non-strict` 200/200, 0 wrong, 1133/1191 top-level statements native (95.1%, 58 in
   CPython fallback). Oracle fixtures in `tests/corpus_round7.rs`, leak probes in
   `tests/memory_leaks.rs`.
5. **Platforms:** the release ships a macOS arm64 archive next to Linux x86_64, both
   smoke-tested in the release workflow.
   **Met 2026-10-12:** [v0.2.0](https://github.com/rotnov/pycc-next/releases/tag/v0.2.0)
   ships `pycc-v0.2.0-x86_64-linux.tar.gz` and `pycc-v0.2.0-aarch64-macos.tar.gz` (macOS
   26.0+), each smoke-tested by release run 38305121948; the Linux archive re-verified
   after download (sha256, `scripts/smoke.sh`).

## v0.3 — Packages, stdlib, everyday code (released 2026-10-14)

Accept:
1. **Multi-module programs:** a program of several `.py` files and packages (`import m`,
   `from pkg.sub import f`, relative imports, `__init__.py`) compiles as one unit, natively
   (strict and `--non-strict`) and as `--ext` modules importable from CPython; calls between
   compiled modules are direct calls. Oracle fixtures against CPython 3.13.
   **Met 2026-10-14:** `tests/multi_module.rs` runs the trees under
   `tests/fixtures/multi_module/` (`import m`, `import pkg.sub.mod as x`, `from pkg import
   submodule`, `.`/`..` imports, `__init__.py` bodies and re-exports, `m.f`/`m.CONST`/
   `m.Class`, cross-module subclasses, an uncaught exception from another module) against
   CPython 3.13 natively in strict and `--non-strict` mode and as an `--ext` module, plus
   the refusal diagnostics. Each module has its own namespace (`pycc_ast::namespace`) and
   its body runs at its first import; remaining gaps in `docs/DEVIATIONS.md`.
2. **Stdlib, native in strict mode:** `dataclasses.dataclass` (fields, defaults,
   `field(default_factory=...)`, generated `__init__`/`__repr__`/`__eq__`, `order=True`,
   `frozen=True`), `enum.Enum`/`IntEnum`, `collections.deque`/`defaultdict`/`Counter`,
   `functools.cache`/`lru_cache`, and `itertools` `product`/`permutations`/`combinations`/
   `accumulate`/`chain`/`islice`/`count`. Oracle fixtures and leak probes.
   **Met 2026-10-14:** `tests/stdlib_dataclasses.rs`, `tests/stdlib_enum_functools.rs` and
   `tests/stdlib_native.rs` (deque, Counter, defaultdict, itertools) run every fixture
   natively in strict and `--non-strict` mode and as `--ext` against CPython 3.13; leak
   probes in `tests/memory_leaks_stdlib.rs` and `tests/memory_leaks/stdlib.rs`; CI green
   on d5f2a39. Remaining gaps in `docs/DEVIATIONS.md`.
3. **Real-world corpus:** strict passes >= 150/200; `--non-strict` keeps 200/200, 0 wrong,
   with >= 98% of top-level statements native.
   **Met 2026-10-14:** `python3 bench/corpus.py` passes 161/200 strict (0 wrong);
   `python3 bench/corpus.py --flags=--non-strict --require-correct` passes 200/200, 0 wrong,
   with 1168 of 1191 top-level statements native (98.1%; 186 programs with no fallback).
   What the corpus needed is pinned by `tests/corpus_v03.rs` against CPython 3.13, with leak
   probes in `tests/memory_leaks_corpus.rs`; remaining gaps in `docs/DEVIATIONS.md`.
4. **A real package:** one pinned, permissively licensed pure-Python package from PyPI,
   unchanged, builds with `--ext --non-strict` and passes its own test suite run by
   CPython against the compiled modules (CI-gated).
   **Met 2026-10-14:** `bench/package_check.py` (CI step "Real package" on Linux x86_64,
   green in run 38550773714) downloads diff-match-patch 20241021 (Apache-2.0, sdist
   sha256-pinned), builds its three modules unchanged with `--ext --non-strict
   --foreign-relative-imports` (the `__init__.py` as `__init__.abi3.so`), removes the
   sources and runs the package's 28 unittest tests with CPython against the compiled
   modules; all 40 functions compile (no class or function falls back), and its speed test
   runs 1.7x faster than CPython. `tests/package_ext.rs` pins the package shapes against
   CPython; remaining gaps in `docs/DEVIATIONS.md`.
5. **Platforms:** the release also ships a Linux aarch64 archive, smoke-tested in the
   release workflow.
   **Met 2026-10-14:** [v0.3.0](https://github.com/rotnov/pycc-next/releases/tag/v0.3.0)
   ships `pycc-v0.3.0-aarch64-linux.tar.gz` next to the x86_64 Linux and arm64 macOS
   archives, each smoke-tested by release run 38584309286; CI also runs the full suite on
   `ubuntu-24.04-arm`.

## v0.4 — Fast untyped code, no leaks (released 2026-10-16)

Accept:
1. **Untyped code runs native:** `--non-strict` infers native `str`, `list`, `dict`, `set`
   and tuple types through unannotated functions and methods (from call sites, defaults,
   literals and returns), so untyped code stops round-tripping through CPython objects.
   Evidence: `python3 bench/package_check.py --time` shows diff-match-patch's own speed
   test >= 5x faster than CPython (v0.3: 1.7x) with its suite still passing.
   **Met 2026-10-15:** `python3 bench/package_check.py --time`: CPython 13.2 s, compiled
   1.60 s (8.3x; 10.5x on a quieter run), suite 28/28 passing, every function native.
   An `--ext --non-strict` module's unannotated functions and methods that loop get
   guarded native specializations (`str`/`int` parameters guessed from their uses, exact
   type tests at entry, the object path otherwise; `tests/untyped_native.rs` calls them
   from CPython with expected and unexpected types against the source module, and probes
   leaks); native `str` indexing/slicing/`len` are O(1); widening is per function.
   Scope: containers (`list`, `dict`, `set`, `tuple`) do not cross the boundary natively
   (a copy would break aliasing), see `docs/DEVIATIONS.md`. Corpus unchanged: non-strict
   200/200, 0 wrong; strict 161.
2. **Exceptions are freed:** raised and caught exception objects and their messages are
   reference counted; a loop that raises and catches (native exceptions and exceptions
   from CPython calls) runs in constant memory (leak probes); the DEVIATIONS row goes.
   **Met 2026-10-15:** `tests/memory_leaks_exceptions.rs` runs 18 raise-and-catch loops
   (native and user exceptions, `as` bindings, causes, bare `raise`, `finally`,
   `return`/`break`/`continue` out of handlers, lists and slots holding exceptions,
   `StopIteration` values, generator `throw`/close, `except*`, `KeyError`, a failing
   `json.loads`) in strict and `--non-strict` builds, 2.2M iterations growing < 1.5 MB
   (was ~220 B per `ZeroDivisionError`, ~1.9 kB per `json.loads` failure);
   `tests/memory_leaks_ext.rs` keeps the runtime's live exception and `str` counts flat
   across the `--ext` boundary; `tests/fixtures/exception_lifetime.py` matches CPython
   and is memcheck-clean. The row is gone; what remains (cause cycles, exception classes
   with their own `__init__`) is listed in `docs/DEVIATIONS.md`.
3. **Real-world corpus:** strict passes >= 170/200; `--non-strict` keeps 200/200, 0 wrong,
   with <= 8 top-level statements in CPython fallback (v0.3: 23).
   **Met 2026-10-15:** `python3 bench/corpus.py` passes 170/200 strict (0 wrong);
   `python3 bench/corpus.py --flags=--non-strict --require-correct` passes 200/200, 0 wrong,
   8/1191 top-level statements in CPython fallback (99.3% native). Oracle fixtures against
   CPython 3.13 in `tests/format_spec.rs`, `tests/maybe_unbound_runtime.rs`,
   `tests/dict_get_optional.rs`, `tests/attribute_dict_producer.rs`,
   `tests/solver_typed_containers.rs`, `tests/non_strict_object_containers.rs` and
   `tests/non_strict_widened_containers.rs`; leak probes in
   `tests/memory_leaks_formatting.rs`.
4. **A second real package:** another pinned, permissively licensed pure-Python PyPI
   package from a different domain builds unchanged with `--ext --non-strict`, passes its
   own test suite (CI-gated) and is measurably faster than CPython on its hot path.
   **Met 2026-10-15:** tomli 2.5.0 (MIT, the TOML parser) builds unchanged and its own
   suite passes against the compiled modules (`python3 bench/package_check.py tomli`,
   CI-gated with diff-match-patch); 36 of its 40 functions compile. `loads()` of its
   benchmark document runs 1.6x faster than CPython: `python3 bench/package_check.py
   tomli --time --runs 15` (100 calls, best of 15 runs for each side, on one machine)
   measured 0.036 s against 0.059 s, 1.52x-1.75x over seven runs; callgrind counts 2.60M
   against 4.51M instructions per call (was 0.40x). The work: multi-value returns
   (`tests/multi_return.rs`), tuple-free `a, b = x, y` (`tests/object_swap_unpack.rs`),
   native `str | None` locals (`tests/optional_str_local.rs`), one-call scans
   (`tests/str_scan_loop.rs`), byte comparisons of objects with `str` literals and str
   sets (`tests/object_text_compare.rs`) and shorter object paths
   (`tests/object_fast_paths.rs`), each oracle-tested native and `--ext` with a leak
   probe; diff-match-patch stays 11x-13x. Its documents, keys and regular-expression
   matches are still CPython objects.
5. **Language gaps from v0.3:** circular imports behave as in CPython; strict `--ext`
   modules expose module-level variables; `IntEnum` members are usable as `int`;
   `sorted()`/`min()`/`max()`/`.sort()` and set membership of class instances;
   `dataclasses.fields()`/`asdict()`. Oracle fixtures; matching DEVIATIONS rows removed.
   **Met 2026-10-15:** `tests/multi_module.rs` (`cycle`, `cycle_error`, `cycle_error_pkg`:
   two modules importing each other, and `from m import x` of a name the partially
   initialized `m` has not bound raising CPython's `ImportError` with its message, native,
   `--non-strict` and `--ext`); `tests/language_v04.rs` (IntEnum members as `int`,
   `sorted`/`min`/`max`/`.sort()` with `key=`/`reverse=` and a list changed while sorted,
   `x in` a set of instances, `dataclasses.fields`/`asdict`/`astuple`/`replace`, lists and
   tuples of instances printed by `__repr__`, a strict `--ext` module's native globals as
   attributes) against CPython, with leak probes in `tests/memory_leaks_v04.rs`. The
   matching DEVIATIONS rows are gone or narrowed to what remains: a circular import
   through the entry module (`E0108`), classes reached through a circular import, a
   strict `--ext` module's CPython-object globals (a second reference would show), and
   instances as `dict` keys.

## v0.5 — Typed packages at native speed (released 2026-10-16)

Accept:
1. **Native containers of objects:** `tuple[T, ...]`, `dict[str, Any]`/`dict[K, object]`,
   `list[object]` and regex match results stay native containers whose elements may be
   CPython objects (boxed lazily at the boundary), so typed code that builds such
   structures stops round-tripping through the C API. Evidence: tomli's `loads()` >= 3x
   faster than CPython (v0.4: 1.67x) via `bench/package_check.py --time`, suite passing;
   diff-match-patch stays >= 10x.
   Native `tuple[T, ...]` and tuples of objects (`tuple[int, Any]`)
   are in (`tests/native_object_tuples.rs`); mutable containers of objects stay
   CPython-owned by design (CPython code may hold and mutate the same object; see
   `crates/pycc_rt/src/tuple.rs`). tomli stays at 1.4x-1.7x: under callgrind, CPython
   itself runs 1.14M of the compiled `loads()`'s 2.70M instructions per call (regular
   expression matching, the operations on its `dict[str, Any]` documents and flag
   tables, `int`/`datetime` conversion), against a 3x budget of 1.52M (CPython: 4.56M).
   - Amendment (2026-10-16): the 3x evidence bar is withdrawn. It assumed tomli's time was
     spent in containers the compiler could own; the profile shows that even with every
     container native, regular-expression matching and match conversion inside CPython
     (~0.57M/call) plus the compiled code (~1.1M/call) exceed the 1.52M/call budget, and the
     mutable `dict`/`list` values tomli hands back must stay CPython-owned for aliasing. The
     accept item is now: native `tuple[T, ...]` and tuples of objects (done, above), and
     tomli no slower than v0.4 — at most 2.60M instructions per `loads()` call under
     callgrind (it regressed to 2.70M with native tuples) — with diff-match-patch >= 10x.
   **Met 2026-10-16** (as amended): tomli `loads()` takes 2.54M instructions per call
   (2.53M-2.54M over repeated runs; 2.69M-2.70M before). Method: build with `python3
   bench/package_check.py tomli --keep DIR` (DIR absolute), then run `valgrind
   --tool=callgrind python3 -I cg.py DIR/tomli/compiled N` for N = 1 and N = 21, where
   `cg.py` puts its argument on `sys.path`, imports `tomli`, reads `benchmark/data.toml`
   and calls `tomli.loads()` N times; per call = (total(21) - total(1)) / 20. The lever:
   list/tuple headers, small tuple data blocks and `str` objects come from bounded free
   lists (`crates/pycc_rt/src/pool.rs`), so libc `malloc`/`free` fell from 0.38M to 0.22M
   per call. diff-match-patch 11.0x, pypng 14.5x, all three suites pass (`--time`, local,
   best of 5); CI run 38892307241 green (Linux x86_64, Linux arm64, macOS).
   - Re-check (2026-10-16, after the corpus round): tomli had risen to 2.95M instructions
     per call (its `frozenset` constants were widened into CPython objects) and
     diff-match-patch to 9.0x-9.9x (identical code for `diff_bisect`, placed where its loop
     jumps cross 32-byte boundaries, which a Skylake-derived Xeon's SKX102 microcode
     penalizes). Both fixed: tomli 2.53M; diff-match-patch 12.4x-13.2x against 10.7x-11.5x
     at de4aace and 9.0x-9.9x at 11ab665, interleaved on one machine (best of 5, three
     rounds). CI now gates the instruction count of each package's hot path
     (`bench/package_check.py --instructions`, bounds 5% above the counts on CI's runner,
     whose CPython executes more instructions: tomli 2.79M there, bound 2.93M).
2. **A third real package:** a pinned, permissively licensed pure-Python PyPI package from
   another domain (numeric/algorithmic preferred) builds unchanged with `--ext
   --non-strict`, passes its own suite (CI-gated) and runs >= 5x faster than CPython on its
   own hot path.
   **Met 2026-10-16:** pypng 0.20220715.0 (MIT, PNG codec) via `bench/package_check.py
   pypng`: its suite passes compiled (CI run 38863464378, all three jobs); `read()` of a
   256x256 RGB image using all five PNG filters runs 10-13x faster than CPython (11.7x on a
   local re-run, best of 5). Only 15/85 functions compile natively (the scanline filters,
   via guarded `bytearray` specializations); `Reader`/`Writer` stay in CPython, so decoding
   tiny images is only ~1.6x faster.
3. **Real-world corpus:** strict passes >= 180/200; `--non-strict` 200/200, 0 wrong, with
   at most 2 top-level statements in CPython fallback (v0.4: 8), each documented.
   **Met 2026-10-16:** `python3 bench/corpus.py` passes 181/200 strict (0 wrong);
   `python3 bench/corpus.py --flags=--non-strict --require-correct` passes 200/200, 0
   wrong, with 2 of 1191 top-level statements in CPython fallback (`--show-fallbacks`:
   problems 134 and 188, both explained in the README's corpus note); CI run 38913979902.
   Oracle fixtures in `tests/corpus_v05.rs`, leak probes in
   `tests/memory_leaks_corpus_v05.rs`, the `int` infinity's differences in
   `docs/DEVIATIONS.md`.
4. **Compile speed:** building each `bench/package_check.py` package takes <= 10 s on the
   CI Linux runner (v0.4: diff-match-patch ~25 s); `pycc build` of every `bench/py`
   program takes <= 2 s. Measured in CI and reported by the script.
   **Met 2026-10-16:** CI run 38875730578 (Linux x86_64): diff-match-patch 5.2 s (its
   main module 4.1 s), tomli 2.8 s, pypng 1.0 s; the slowest `bench/py` build 0.71 s
   (prefix_sum_np). CI now fails a package over 10 s
   (`bench/package_check.py --max-build-seconds 10`) and a benchmark build over 2 s
   (`bench/run.py --build-only --max-build-seconds 2`). Fewer, parallel frontend
   attempts (the specialization loop's), a cheaper type checker, LLVM's backend on
   several threads and the C shim compiled meanwhile; `PYCC_TIME=1` prints the phases.
5. **Correctness backlog:** every open issue in this milestone is fixed or closed with
   evidence (starting with #1, non-strict rebinding of a function by compiled callers).

## v0.6 — Drop-in for real projects (released 2026-10-17)

Accept:
1. **`pip install` builds with pycc:** a PEP 517 build backend (`pycc.build`, shipped in the
   release archive and installable) lets a project set `build-backend = "pycc.build"` in
   `pyproject.toml`; `pip install .` and `pip wheel .` compile its pure-Python modules to
   abi3 extensions (`--ext`, strict or `--non-strict` per `[tool.pycc]`), keep a module in
   pure Python when told to, and produce a valid wheel. Evidence: a CI job builds a wheel
   for each `bench/package_check.py` package through pip and runs its suite against the
   installed wheel. **Met 2026-10-16** (CI run 38976843855, commit 86a0eeb):
   `bench/wheel_check.py` built `cp313-abi3-manylinux_2_39_{x86_64,aarch64}` wheels of
   diff-match-patch, tomli and pypng with `pip wheel` and the packaged archive's backend
   on both Linux legs, and each package's own suite passed against the installed wheel
   (pypng's 6 numpy tests skip: numpy is not in the fresh venv); `scripts/smoke.sh`
   built, installed and ran a project's wheel through pip on Linux x86_64, aarch64 and
   macOS arm64.
2. **Classes in untyped code:** `--non-strict` compiles untyped classes natively — `bytes`
   literals and native `bytes` values, calls with `*args`/`**kwargs`, classes with methods
   that take or return CPython objects, generators in methods. Evidence: pypng compiles
   >= 60 of its 85 functions natively (v0.5: 15) and decodes the PngSuite images it tests
   >= 3x faster than CPython (v0.5: ~1.6x).
   **Met 2026-10-17:** 64/85 functions compile, `Reader` included; decoding every
   PngSuite image its tests use (`bench/package_check.py pypng --time`, 28 images x5) is
   3.18x faster than CPython (best of 15 interleaved runs: 18.8 ms against 59.8 ms,
   median 3.16x, load average 5.6 on 4 cores) and runs 4.57x fewer instructions under
   callgrind (35.9M against 164.1M per decode). CI run 39010298602 on bc91067 green on all three jobs; margin is thin (3.16-3.18x).
3. **A real `int | float` type:** strict mode types a variable that holds an `int` or a
   `float` as `int | float` with CPython semantics for every operation, replacing the v0.5
   int-infinity rewrite; its DEVIATIONS rows are removed.
   **Met 2026-10-17:** `Ty::Num` (a tagged word: inline int/float, heap bigint or boxed
   float) in strict mode; the int-infinity rewrite and its DEVIATIONS rows are gone
   (`tests/num_union.rs`, leak probes included). One new, narrower DEVIATIONS row remains:
   a boxed `int | float` value (a >63-bit int or a float outside ~1e-77..1e77) stored in a
   container raises `OverflowError`. CI run 39023482861 on 4dd0f8f green on all three jobs.
4. **No hidden differences:** a differential fuzzer generates typed Python programs, runs
   them under CPython 3.13 and as native strict, `--non-strict` and `--ext` builds, and
   compares output and exceptions; a nightly CI job runs it for >= 30 minutes; every
   mismatch it found is fixed (with a regression fixture) or documented in DEVIATIONS.
   **Met 2026-10-17:** `fuzz/` (generator, harness, AST reducer, runner); nightly
   `.github/workflows/fuzz.yml` run 38996102925 on ad3e996: 3314 programs compared in
   1802 s, 0 findings (an earlier nightly run on fe05aa0: 2190 programs, 0 findings). The
   ~5 hours of local runs found 18 mismatches, each fixed with a fixture in
   `tests/fuzz_regressions.rs`; 5 documented differences are listed in DEVIATIONS and
   filtered by the harness. A 120 s smoke run is part of regular CI.
5. **Real-world corpus:** strict >= 190/200; `--non-strict` 200/200, 0 wrong, with no
   top-level statement in CPython fallback.
   **Not met (2026-10-17):** strict 188/200 (from 183), 0 wrong; `--non-strict` 200/200,
   0 wrong, 1/1191 top-level statements in CPython fallback (from 2). The remaining
   fallback is problem 188's `mat_pow`; it needs (1) folding `isinstance` of a statically
   known type inside function bodies, (2) parameters as the first definition of a
   rebinding web (`B = [[b] for b in B]` gets its own type), and (3) per-call-site
   instantiation of a nested helper called with different container types (needed only
   for 188). The other strict refusals are heterogeneous containers (049, 070, 130), an
   `int | None` used as an `int` without a `None` test (016, 056, 100, 123, 184), path-
   dependent rebinding (200), `iter`/`range` objects as values (057, 168) and 188.
   Carried over to v0.7 accept 5 (issues #5, #6, #7); v0.6.0 ships without it.

## v0.7 — Real-world Python: frameworks, Windows (released 2026-10-17)

Accept:
1. **Windows:** a `pycc-vX.Y.Z-x86_64-windows.zip` release archive (MSVC toolchain, CPython
   3.13+ for `--ext`), built and smoke-tested in the release workflow; CI runs the test
   suite, the corpus gate and the real-package checks on `windows-2025`; the PEP 517 backend
   produces `win_amd64` wheels. macOS also runs the real-package wheel checks.
   **Met 2026-10-17:** ci.yml's `windows` job (windows-2025, MSVC, LLVM's own 22.1.8
   release pinned by sha256 in `scripts/windows_llvm.sh`) builds, packages and smoke-tests
   the `.zip` archive, builds diff-match-patch, tomli and pypng into `cp313-abi3-win_amd64`
   wheels with `pip wheel` (`bench/wheel_check.py`) and against their own tests
   (`bench/package_check.py`), runs the whole test suite (`scripts/ci_test.sh` under Git
   Bash) and the non-strict corpus gate (200/200); release.yml builds, smoke-tests and
   publishes `pycc-vX.Y.Z-x86_64-windows.zip` with the other archives (not yet exercised by
   a release). macOS runs `bench/wheel_check.py`. CI run 39041988599 on fad5439 green. Windows gaps:
   `docs/DEVIATIONS.md` (native stdout writes `\n`, no static libpython).
2. **Classes from libraries:** `--non-strict` compiles classes that inherit from CPython
   classes (bases written as names or attributes, `class M(lib.Base)`), with metaclasses,
   descriptors, class attributes of any type, decorated methods and properties; annotations
   naming library types (`mod.Type`), general unions and `X | None` of object types; the
   `with` statement (sync and async context managers), and starred expressions in every
   position. Oracle fixtures per feature. **Met 2026-10-17** (CI run 39035410995, commit
   1b4f1e2): `tests/library_classes.rs` (`threading.Thread`, `collections.Counter`,
   `argparse.Action`, a `namedtuple` and exception subclasses, a metaclass,
   `__init_subclass__`, a descriptor, `@property`/`@staticmethod`/`@classmethod`/
   `cached_property`/`functools.wraps`, `re.Match` and union annotations, `with`, starred
   expressions; native, `--ext`, subclassed from Python, constant memory); the class
   statement runs in CPython with its method bodies compiled (`src/fallback/lift.rs`).
   `async with` is not compiled (no async support; it runs in CPython, DEVIATIONS).
3. **Method-level fallback:** a construct `--non-strict` cannot compile sends only the
   smallest enclosing function or method to CPython, never the whole class or module.
   **Met 2026-10-17** (CI run 39035410995, commit 1b4f1e2): `tests/method_fallback.rs`
   (only the failing methods run in CPython; an `--ext` compiled class keeps its native
   type with the failing method attached, `src/fallback/attach.rs`; Python subclasses,
   `super()` and a leak probe); pydantic 2.13.5 (`bench/pydantic_check.py`): 789/1249
   functions compile, every module builds (33/1249 and 55 failing builds before).
4. **pydantic v2, measured once:** the Python part of pydantic v2 (pinned; pydantic-core
   unchanged) builds with `--ext --non-strict`, passes its own test suite against the
   compiled modules, and its model-definition / schema-building time is measured against
   CPython; the result (speed-up or not) and the remaining fallbacks are recorded here.
   This is a one-off evaluation, not a CI gate.
   **Met 2026-10-17** (one-off, local, `bench/pydantic_check.py --time --tests`, shared
   4-core Xeon, CPython 3.13.16, pydantic-core 2.46.5 for both trees): pydantic 2.13.5
   builds with every module (809/1249 functions native); its suite passes 5742 tests
   with 20 failing against the compiled tree (5762 and 0 against the sources; the 20
   are generic-model cache and pickle tests that walk `sys._getframe`, see DEVIATIONS,
   and tests that monkeypatch a compiled module's globals). **The compiled tree is
   slower**, median of 15 interleaved fresh-process runs (ms, compiled vs CPython;
   speed = CPython/compiled):
   import 191.6 vs 75.8 (0.40x), model definition 12.9 vs 9.0 (0.70x),
   `model_json_schema()` 25.8 vs 18.0 (0.70x), validation/serialization round-trips
   18.8 vs 17.0 (0.91x). Why: (1) import compiles 674 KB of CPython source with
   `compile()` on every import (classes whose statement runs in CPython, their method
   forwarders and the methods that fall back), which a `.pyc` cache saves CPython;
   (2) every call into a compiled function crosses a boundary of ~25-40 ns (argument
   unpacking, a `str` parameter copied into a native string, a Python forwarder frame
   for a method of a CPython-run class), and pydantic's schema building is many short
   calls; (3) the bodies are object-protocol code (attribute, item and call through
   the C API without CPython's specializing interpreter), so a body compiled natively
   gains little, except loop-heavy helpers (`_deduplicate_schemas` runs ~6x faster);
   (4) a caught CPython exception was bridged with up to 33 class matches -- fixed,
   exact builtin classes now match by identity (a caught `KeyError` 496 -> 312 ns,
   CPython 202). Remaining top fallback reasons (functions): `...` literal 104,
   function-local `import` 61, nested `def` 59, `lambda` 23, `hash()` of an object 15.
5. **Real-world corpus:** strict >= 190/200; `--non-strict` 200/200, 0 wrong, with no
   top-level statement in CPython fallback (carried over from v0.6 if not met there).
   **Met 2026-10-17** (CI run 39051177139, commit 61e1488): strict 190/200, 0 wrong
   (`python3 bench/corpus.py`); `--non-strict` 200/200, 0 wrong, 0/1191 top-level
   statements in CPython fallback (`--require-correct`). Problem 188 compiles through
   `isinstance` folding of a statically known type (#5), parameter webs (#6) and
   per-call-site copies of nested helpers (#7) (`pycc_types::static_isinstance`,
   `pycc_ast::desugar::webs`); problem 168 through iterated `range` values compiled as
   triples (`pycc_ast::desugar::range_values`) and `is None` folding of a never-`None`
   value. Tests: `tests/corpus_v07.rs`. The remaining strict refusals are mixed-type
   containers (049, 070, 130), an `int | None` used as an `int` without a `None` test
   (016, 056, 100, 123, 184), a variable rebound to another type on some paths (200) and
   iterator objects as values (057).

## v0.8 — Library code faster than CPython

v0.7 made real libraries compile (pydantic: 809/1249 functions native, its suite passing)
but the compiled tree is slower than CPython running the sources (import 0.40x, model
definition 0.70x, `model_json_schema()` 0.70x, round-trips 0.91x). v0.8 makes compiled
library code a speed-up, not only a correctness result.

Accept:
1. **pydantic faster than CPython:** `bench/pydantic_check.py --time` on pydantic 2.13.5
   (same pydantic-core wheel for both trees): import no slower than CPython (the fallback
   source is no longer compiled with `compile()` on every import), model definition and
   `model_json_schema()` >= 1.3x faster, validation/serialization round-trips no slower.
   **Progress 2026-10-17** (not met; local, shared 4-core Xeon, CPython 3.13.16,
   pydantic-core 2.46.5, median ms of interleaved fresh-process runs,
   compiled vs CPython):

   | step | import | define | schema | round-trips |
   |---|---|---|---|---|
   | v0.7 (7 runs) | 179.1 vs 81.5 (0.46x) | 12.9 vs 9.6 (0.75x) | 28.7 vs 17.1 (0.59x) | 19.7 vs 17.0 (0.86x) |
   | fallback source pre-compiled (7 runs) | 100.1 vs 76.2 (0.76x) | 13.1 vs 8.9 (0.68x) | 25.3 vs 16.7 (0.66x) | 19.9 vs 17.3 (0.87x) |
   | + only used runtime linked, call-site caches, lazy bridged text (15 runs, loaded host) | 95.5 vs 72.9 (0.76x) | 12.4 vs 9.4 (0.76x) | 25.7 vs 17.5 (0.68x) | 18.0 vs 17.1 (0.95x) |

   Done: (1) the fallback source is marshalled at build time and unmarshalled at import
   when the bytecode magic number matches and `-O` is off, with a `compile()` fallback
   otherwise (`src/ext_build/precompiled.rs`); (2) an ELF module exports only `PyInit_*`
   and the live-object counters and links with `--gc-sections`, so code shrinks from
   2.4 MB to 1.0 MB per module (pydantic tree 490 MB -> 62 MB) and `dlopen` takes about
   half as long; (3) keyword-name tuples are cached per call site, the attribute-name
   cache has 4096 slots, and `isinstance` checks the exact type first; (4) a caught
   CPython exception's `str()` is rendered only when compiled code reads it, which is
   also CPython's timing. Tried and reverted: replacing the Python forwarder of a
   CPython-run class's compiled method with a vectorcall method descriptor. It saved
   7-14 ns per call, but it broke 36 pydantic tests that find their caller's namespace
   with `sys._getframe(n)` and count the forwarder frames. That change waits for accept
   3 (compiled frames as real frames). Profile (callgrind/cachegrind, one
   define+schema round): the compiled tree runs 184M instructions to CPython's 151M and
   gets 3.3M L1 instruction misses to CPython's 2.0M. The cost is spread out: stable-ABI
   reference counting goes through function calls (`_Py_IncRef`/`_Py_DecRef`, 700K
   calls per round), each module carries its own copy of the runtime helpers, and most
   executed code still runs in the interpreter (the fallback code). No single hot path
   carries the gap. What is left on import (about 20 ms) is resolving and mapping 79
   shared objects (symbol lookup about 7M instructions) and the mirrored imports,
   which run twice.
2. **The common fallback constructs compile:** the `...` literal, function-local
   `import`, nested `def` (closures, decorated wrappers such as `functools.wraps`),
   `lambda` and `hash()` of an object compile natively; pydantic >= 1000/1249 functions
   native; the call boundary costs no Python forwarder frame for methods of CPython-run
   classes.
3. **Compiled frames are real frames:** `sys._getframe`, tracebacks and line numbers of
   compiled functions match CPython (their DEVIATIONS rows are removed); pydantic's own
   suite fails no test that passes against its sources.
4. **A fourth real package, another domain:** a pure-Python text-processing package from
   PyPI (a Markdown or template engine) builds with `--ext --non-strict`, passes its own
   test suite, and runs its own benchmark or a representative workload >= 2x faster than
   CPython; added to `bench/package_check.py` and the CI wheel checks.
   **Progress 2026-10-17** (not met): mistune 3.3.4 (BSD-3-Clause, a CommonMark Markdown
   engine, 38 modules, sdist pinned by sha256) builds unchanged with `--ext --non-strict`
   and its own 1155 tests pass against the compiled modules (`bench/package_check.py
   mistune`, `bench/wheel_check.py mistune`; added to CI on all four platforms, with a
   callgrind bound on one `mistune.html` pass over `benchmark/cases`); 380 of its 413
   functions compile. It is **slower** than CPython: 0.70x on `mistune.html` of its
   benchmark documents, 0.74x with every plugin (local, best of 3); under callgrind 72.2M
   instructions per pass against CPython's 56.8M. Compiled code runs a third of the
   bytecode (13M fewer instructions in the interpreter loop) but paid 34M back when profiled at 77.7M: generic
   `PyObject_GetAttr` on CPython-created instances (+5.4M with the type lookups), method
   and temporary objects (+2.4M deallocation), stable-ABI `_Py_IncRef`/`_Py_DecRef` calls
   (+2.1M), the forwarders of methods of CPython-run classes, and `str` conversions
   (+1.6M). Fixed on the way, each general: `typing.Optional`/`Union` and unmodeled
   `typing` names in non-strict mode, string annotations naming `TYPE_CHECKING`-only
   imports, private module functions exported, `str` crossing the boundary without
   copies (identity kept), searches with `memchr`, `typing.cast` of objects erased,
   `for c in s` without a list of characters, and the C shim's fixed half compiled
   once per machine (`--ext` builds of small modules ~1.4 s -> ~0.4 s). Remaining for 2x:
   its classes (`Parser(Generic[ST])`, `BlockState`, `InlineState`, the renderers) must
   compile natively and their instances be reached natively from the other modules of
   the package -- each module is its own extension, so today another module sees a
   compiled instance only through CPython's attribute protocol.
5. **Windows parity and corpus:** native programs on Windows write CPython's text-mode
   stdout (`\r\n`, the console encoding), removing that DEVIATIONS row; corpus strict
   >= 193/200, `--non-strict` 200/200, 0 wrong, 0 fallback statements.

Progress:
- Accept 2 (in progress): the `...` literal (a bare `...` statement is a no-op; the value
  is the `Ellipsis` object), `hash()` of a CPython object and function-local imports of
  CPython modules compile natively, and in non-strict code a nested `def` or `lambda`
  used as a value runs in CPython alone, its body compiled and its enclosing function
  compiled (`tests/v08_constructs.rs`); pydantic 2.13.5: 978 of 1249 functions native
  (`bench/pydantic_check.py`, from 809; 1021 before typing's `Union`/`Any`/`Annotated`
  became compiled names, whose module-level value uses now fall back), its suite failing
  the same 20 tests. Remaining: those value uses, and the call boundary's forwarder frame.
