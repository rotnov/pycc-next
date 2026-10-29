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

## v0.8 — Library code faster than CPython (released 2026-10-21)

v0.7 made real libraries compile (pydantic: 809/1249 functions native, its suite passing)
but the compiled tree is slower than CPython running the sources (import 0.40x, model
definition 0.70x, `model_json_schema()` 0.70x, round-trips 0.91x). v0.8 makes compiled
library code a speed-up, not only a correctness result.

Released as 0.8.0 with accepts 2 and 3 met and accepts 1, 4 and 5 not met (Windows
parity holds; the corpus stops at 191/200 strict). Accepts 1, 4 and 5's corpus half are
carried into v0.9 as reshaped goals (see v0.9).

Accept:
1. **pydantic faster than CPython:** `bench/pydantic_check.py --time` on pydantic 2.13.5
   (same pydantic-core wheel for both trees): import no slower than CPython (the fallback
   source is no longer compiled with `compile()` on every import), model definition and
   `model_json_schema()` >= 1.3x faster, validation/serialization round-trips no slower.
   - Amendment (2026-10-21): the 1.3x define/schema bar is withdrawn. It assumed
     compiling pydantic's own code would make it faster than CPython interpreting it;
     the profile says what is compilable is too small a share, and stays object-protocol
     work. Evidence (callgrind, one define + schema repetition, (5 reps - 1 rep) / 4):
     CPython on the sources runs 151.2M instructions, 97% inside the interpreter binary
     -- 57.6M in `_PyEval_EvalFrameDefault`, ~90M in C API object operations; the
     compiled tree runs 171.0M -- 33.5M in compiled code, 21.2M still interpreted
     (stdlib `typing`/`inspect`/`dataclasses` and pydantic's fallback code), ~116M of C
     API object operations and libc (more than CPython's: temporaries' deallocation
     5.1M against 1.9M, attribute-name and pack/release helpers). By owner (cProfile
     tottime on CPython): pydantic's own Python ~48%, builtins/C object work ~30%, the
     standard library ~16%, import machinery ~5%. 1.3x means removing ~23% of CPython's
     total from the ~48% that is compilable, so pydantic's code would have to run ~1.9x
     faster than CPython interprets it; it is metaprogramming over CPython objects
     (classes `ModelMetaclass` creates, `typing` objects, dicts), each operation an
     object-protocol call compiled or not, and its compiled form is slower than the
     interpreter today. No lever pycc can build reaches 1.3x. The accept item is now: no
     phase slower than CPython -- import, define, schema and round-trips each >= 1.0x
     (median ms of interleaved fresh-process runs, `bench/pydantic_check.py --time`),
     reached by cutting pycc's own per-operation cost on CPython objects (temporaries'
     deallocation, the attribute-name cache path, pack/release) and import's remaining
     extension creation and module execution, each step measured; Met only when it
     holds on CI.
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
   **Progress 2026-10-18** (not met). Design, agreed with the guru advisor: the shared
   root cause of accepts 1 and 4 is that library classes are created by CPython and that
   every module is its own extension. Three steps, cheapest first: (a) inline reference
   counting -- done: `ob_refcnt` at offset 0 and `_Py_Dealloc` are stable ABI, so a
   `--release` `--ext` artifact keeps its `abi3` tag and counts inline with CPython
   3.13's 64-bit GIL forms (`crates/pycc_codegen/src/object_rc.rs`, the shim's
   `bridge.c`; a cp313-only artifact is not needed); (b) the per-class reasons classes
   fall back -- done so far: annotated attribute stores, `self.__class__(...)`,
   non-strict declarations without a slot, a published method returning `T | None`
   (through an entry returning a CPython object), and container slots that code outside
   the class reads through CPython kept as CPython objects (needed for correctness once
   more classes are native); still open: a constrained `TypeVar` in a `Generic[...]`
   base (to be monomorphized per constraint or erased to `object` in non-strict mode;
   mistune's `Parser` also has subclasses in other modules, so only (c) makes it
   native), lambdas in bodies; (c) a whole-package build: every module of a package compiled into
   one extension (`pkg/__init__.abi3.so`, one runtime copy, one `dlopen`, native calls,
   attribute access and subclassing across modules), each submodule published under its
   own name with its own module object and globals by a `sys.meta_path` finder the
   extension owns (`find_spec` only for names in its table, `create_module` returning the
   pre-created module, `exec_module` running the compiled body, so CPython's lazy import
   order, `sys.modules`, `__spec__`/`__file__` and reload behave), staged as per-module
   fallback inside the whole-program build first, then the finder, then the PEP 517
   backend and wheel checks. Rejected: one trampoline `.so` per submodule (a shared-library
   dependency per module on four platforms for no gain) and PEP 697 subclassing of
   another extension's type (it needs a cross-extension layout ABI and still calls the
   base's methods through `PyObject`). Measured after (a) and (b), on main with compiled
   frames (local, medians of 9 interleaved fresh-process runs, compiled vs CPython):
   define 11.8 best / 14.0 median ms vs 8.7 / 9.4 (0.67x; 0.60x before), schema 21.7 /
   24.2 vs 16.3 / 17.0 (0.70x; 0.68x), round-trips 0.95x, import 0.74x; 1025 of 1249
   functions native (1029: three methods of a class that now stays native raise an
   exception class with its own `__init__`, which runs in CPython), the suite passing.
   Import, measured for (c) (local, best of 9): of `import pydantic` and the names the
   workload uses, 103 ms against CPython's 72 ms, creating the 49 extension modules
   (`dlopen` and `PyInit_`) is 7.8 ms; the rest is running their bodies. So linking the
   per-module builds into one shared object alone would buy at most that, and nothing on
   define/schema or mistune. The advisor's refinements to (c): if that link step comes
   first, it is (c)'s first commit in (c)'s shape (symbols isolated at emission --
   every non-`PyInit_` symbol internal or hidden, the shim's file-scope definitions
   `static` -- not localized after the link; one `pkg/__init__.abi3.so`; the finder, the
   build plumbing and the wheel checks (c) keeps); then each module needs its own globals
   dict for the frames' code objects and `f_globals`, its full identity set (`__spec__`,
   `__file__` inside the package directory, `__loader__`, PEP 562 `__getattr__`),
   imports resolved at exec time through `sys.modules` with a native cross-module call
   guarded by the dict's version tag (cycles keep CPython's partially initialized
   modules), per-module shim state (carrier types keyed by module and class), types
   created in `exec_module`, `__module__`/`__qualname__` for pickling, each module's
   precompiled fallback run in its own dict, and the compile time of one large LLVM
   module measured early (per-module LLVM modules in one artifact, calling each other
   through symbols, is the fallback shape).
   **Progress 2026-10-20** (not met): (c)'s first stage landed -- the packaging. `pycc
   build PKG --ext -o PKG/__init__` compiles every module of the package into one
   `PKG/__init__.abi3.so`: each module by a member build of its own, in parallel and
   with its per-module code unchanged (its own LLVM module, its own C unit), then one
   runtime unit, one package unit (`src/ext/package/finder.c`: `PyInit_<pkg>`, the
   `sys.meta_path` finder, `PyModule_FromDefAndSpec`/`PyModule_ExecDef` per module)
   and one link. Decisions, with the guru advisor: symbols are isolated where they
   are emitted (`pycc_codegen::member_symbols`: every symbol a member defines, and the
   module-unit entry points it calls, renamed `pycc_P<len><part>..._` after
   optimization and before the split emission, whose promoted locals take the same
   prefix; the member's C unit reads the same names from a generated header and makes
   its own definitions `static`), not by `ld -r`/`objcopy`; the shim's per-module
   state is an explicit record per module (`PyccExtModule`: name, class tables,
   carrier types, home module, forwarder slots, exec keys), passed by the module
   unit's entry points, not ambient; frame globals are bound per function record at
   each exec; user exception classes share one registry with package-unique tags
   (each member's tag base estimated from the sources and checked after its build),
   and `except` in one module matches another module's classes through a
   class-based fallback of the tag test (as CPython's `isinstance`). Measured
   (local, shared 4-core Xeon, 15 interleaved fresh-process runs, best / median ms):
   pydantic built whole against one extension per module, import 77.0 / 96.2
   against 92.3 / 115.3 (CPython 59.7 / 73.3), define 10.1 / 12.8 against 10.6 /
   13.9, schema 17.5 / 20.9 against 19.5 / 22.9, round-trips 16.5 / 19.6 against
   15.7 / 20.1; 1025 of 1249 functions native; its suite fails the same 4 tests as
   against its sources (`test_exports`, which import the package in a subprocess
   without it on the path) and passes 5758. So the import gains about 15 ms
   (more than the 7.8 ms of extension creation measured before: one `dlopen`, one
   runtime mapped), and the rest is unchanged, as the code each module runs is.
   **Progress 2026-10-21** (not met): (c)'s second stage -- native calls between the
   modules. Decisions, with the guru advisor: each member build writes the interface of
   what it compiled (its published module functions of scalar and object signatures,
   `crate::package_build::interface`) once its MIR exists, before codegen, and the
   package build starts a member once the interfaces of the members it imports from at
   module level exist (not under `if TYPE_CHECKING:`; members of one import cycle keep
   calling each other through CPython), the busiest producers first; each consumer
   records the content hash of what it read and the link refuses a mismatch. In a
   non-strict `--ext` member every module-level import runs in CPython (an imported
   name is a module variable Python code reads), so the guard is on the consumer's own
   dict entry, synced through the dict's version tag: a call `f(a, b)` with exactly
   `f`'s parameters becomes a call of a synthetic `_pycc_sib_f` with `f`'s compiled
   signature whose body calls the object, and codegen picks `f`'s compiled function
   while the dict binds the object `f`'s module published (its interface table,
   refilled by a reload) -- one indirect call, arguments marshalled once. A statement
   that a call's native result breaks (a native `str` among objects in a display) is
   compiled again without the rewrite before it may fall back. Measured: 2M calls of
   another module's `int` function from a compiled loop 0.69 s -> 0.05 s (CPython 0.10
   s); pydantic built whole (15 interleaved runs, best / median ms, against the first
   stage; the numbers first recorded here had measured the first stage's binary
   twice): import 81.4 / 98.9 against 85.7 / 100.3, define 10.4 / 12.7 against 9.9 /
   12.5, schema 18.5 / 23.9 against 17.4 / 22.3, round-trips 17.8 / 20.2 against
   17.5 / 20.8 (CPython 62.0 / 82.5, 7.3 / 8.9, 12.5 / 16.3, 14.9 / 18.7): unchanged,
   its suite as before. 1029 of 1249 functions native again: an exception class with
   a constructor of its own now runs in CPython itself when compiled code raises it
   (the functions raising and catching it stay compiled), `isinstance` of a caught
   exception against a builtin exception class is CPython's test (it answered `False`
   for a CPython exception object before), and compiled code passing a CPython object
   to a parameter annotated with a class compiled in the module converts it to the
   carrier's instance (`TypeError` otherwise) instead of the caller falling back. What
   runs hot in both packages is methods of classes CPython runs (accept 4's note): the
   next stage is native instances and classes across modules.
   **Progress 2026-10-21** (not met): (c)'s third stage, first part -- native instances
   of another module's classes. Decisions, with the guru advisor: instances first (they
   pay in the lifted method bodies already, and the classes of the next part need them),
   then other modules' classes as bases, by compiling the base's methods into the
   derived class as D-254 already compiles inherited methods, with a real CPython base
   on the derived type. The interface lists each compiled class with no base and no
   subclass (an instance typed as it has its layout): its slots in layout order and
   types, its compiled methods' signatures. A consumer importing it at module level gets
   a *sibling class* `_pycc_sibc_C` of the same layout (an `__init__` assigning the
   slots in order) whose methods are declared and never compiled: codegen calls the
   producer's `pyfn_C.m`, and the conversions between an instance and a CPython object go
   to the producer's entries under the class's own name (`isinstance`, unboxing,
   coercion: the producer owns the carrier type); an instance packed into CPython finds
   the carrier type of the module owning its class through the extension's list of
   module records. An annotation naming the imported class resolves to the sibling class
   (`pycc_hir::set_sibling_class_names`), in compiled functions and in the compiled
   bodies of methods whose `def` CPython runs. Soundness: after the type check the
   sibling class's slots and method signatures must equal the interface's, or that
   class is read through CPython again; a failure inside a function taking such a
   parameter makes that function read it through CPython (one more attempt), as stage
   two does for calls. mistune: 63.33M instructions per `mistune.html` pass (64.51M),
   387 of 413 functions native, its suite passing; pydantic unchanged (1029 of 1249,
   its suite as before), its classes being created by CPython.
   **Progress 2026-10-21** (not met), toward the amended bar: the per-operation lever,
   first step -- CPython 3.13's attribute specializations as runtime caches (with the
   guru advisor: the guards of `LOAD_ATTR_INSTANCE_VALUE`/`_SLOT`/`_PROPERTY`/
   `_METHOD_*`/`_MODULE`, per attribute name and class version tag, layouts checked at
   first use; `src/ext/module/attr_cache.c`), `o == "lit"` read in place for a compact
   ASCII `str`, and a set display of literals in a membership test built once per site
   (it was a set and a frozenset per test: 2.3M of define's instructions). Callgrind,
   (5 reps - 1 rep) / 4, `PYTHONHASHSEED=0`: define 41.6M -> 37.9M per repetition
   (CPython 34.1M, 0.90x); define+schema 167.4M -> 157.1M (CPython 151.2M, 0.96x). Wall
   time moves less than instructions (12 interleaved runs, REPEAT 40: define 11.84 ->
   11.12 ms, schema 21.37 -> 20.91 ms; CPython 9.05 / 16.74 ms in the same session:
   about 0.81x both): the compiled tree also misses the instruction cache more
   (cachegrind: 3.15M L1i misses per define+schema round against CPython's 1.99M).
   Then: a method call whose arguments run code loads the method first as `LOAD_ATTR`
   does (function plus receiver, no bound method); a fallback chunk holding a
   `frozenset` constant ships pre-compiled, each constant rebuilt at import from its
   source order as the compiler builds it (pydantic compiled `json_schema`'s 73 KB
   chunk at every import). Per lever, callgrind instructions per repetition:

   | lever | define | define+schema | import (whole process) |
   |---|---|---|---|
   | start (CPython) | 41.6M (34.1M) | 167.4M (151.2M) | 519.6M (442.9M) |
   | attribute caches, in-place str compare | -1.4M | -10.3M with the next row | -- |
   | set display of literals built once per site | -2.3M | (above) | -- |
   | method loaded before its arguments | -0.1M | -2.3M | -- |
   | `frozenset` chunks pre-compiled | -- | -- | -41.4M |
   | one marshalled table of chunks per module | -- | -- | -5.2M |
   | forwarders: interned attribute names, a function's `__dict__` read in place (none created) | -- | -- | -4.1M |
   | the nested-function helpers chunk pre-compiled | -- | -- | -11.2M |
   | the `frozenset` fixer shipped compiled in the table | -- | -- | -2.0M |
   | tried, reverted: hot runtime functions grouped (`__attribute__((hot))`) | L1i misses unchanged (0.17M vs 0.18M) | | |
   | tried, reverted: a forwarder's slot called directly from the cached call | no gain (the slot lookup cost more; such calls mostly come the bound-method way) | | |
   | end | 37.8M (0.90x) | 154.8M (0.98x) | 455.7M (0.97x) |

   Wall time, median of interleaved fresh-process runs (`bench/pydantic_check.py`'s
   timing loop, 11 runs; CPython vs compiled): import 86.8 vs 90.4 ms (0.96x), define
   8.75 vs 12.30 ms (0.71x), schema 16.64 vs 20.59 ms (0.81x), round-trips 18.71 vs 20.46
   ms (0.91x); a second session (20 runs, REPEAT 30) gave import 0.90x, define 0.77x,
   schema 0.84x. Assessment, with the guru advisor: define and schema do not reach
   1.0x, and no lever left in this architecture can. Per define+schema repetition the
   extension's own code is ~43M instructions (7.75M compiled bodies, 18.8M of runtime
   helpers, the caches, forwarders and wrappers) and libpython's work (114M) is about
   CPython's non-interpreter share, so the compiled code replaces the interpreter's
   specialized bytecode at roughly 1.1x its instruction count, with a larger footprint
   (L1i misses 0.88M against 0.62M per define round, D1 0.41M against 0.32M; a cycle
   model from cachegrind's counts gives 0.83x, as measured). 1.0x needs ~20% fewer
   cycles: the extension's share would have to fall from ~43M to under ~10M while the
   fixed libpython and pydantic-core work stays -- the object operations themselves
   would have to disappear, which pydantic's metaclass- and dict-driven definition
   code does not let an ahead-of-time compiler do; CPython 3.13's specializing
   interpreter is already inline-cached C with no marshalling per operation. What
   would meet the bar is a typed native path for the definition machinery (field,
   decorator and config collection as native dicts and lists with compiled `str` keys),
   strict-mode work for another milestone. Import, after the four import rows of the
   table (31 interleaved fresh-process runs, median / best ms, CPython vs compiled):
   73.9 / 58.8 vs 77.2 / 62.3 (0.96x) with the collector on, 67.0 / 52.8 vs 71.4 / 57.5
   (0.94x) with it off -- short of 1.0x. The mirrored imports (an import a compiled
   module binds natively runs again in CPython when fallback code reads its name) turned
   out cheap: 296 such chunks at 0.3-0.5 us each. The 12.8M instructions left are
   spread thin: 8.8M of libpython work on the extension's behalf (attribute reads by
   name, module-dict stores, object setup -- no function above 1.7M), 2.0M of the
   extension's own code, 0.55M of relocations, and the extension (10.5 MB) is mapped
   and faulted in where CPython reads small `.pyc` files.
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
   instructions per pass against CPython's 56.8M (83.2M on CI's x86 runner, bound 85.4M,
   CI run 39078886178, where Linux and macOS are green and Windows builds and passes
   all four packages). Compiled code runs a third of the
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
   **Progress 2026-10-18** (not met): with compiled frames (accept 3) and inline
   reference counting 0.78x on `mistune.html`, 0.85x with every plugin (local, best of
   5; 0.68x and 0.64x on main before the inline counting), 66.2M instructions per pass
   with every release inlined (68.7M before it); 64.8M and 0.84x / 0.78x (best of 5) with
   `BlockState` native too and the release kept one call (inlining it doubled LLVM's time
   and broke CI's 10 s package build bound, for 0.46M instructions).
   386 of 413 functions native; `BlockState` and `InlineState` are native types
   (annotated attribute stores, `self.__class__(...)`, non-strict declarations without a
   slot, `Optional[int]` results published through an object-returning entry, accept 1's
   note). Making `InlineState` native changed nothing measurable on its own (69.65M
   against 69.67M): the instances are reached from the parser modules, which are other
   extensions, through CPython. That, and the
   parsers' `Generic[ST]` and cross-module bases, are what the whole-package build of
   accept 1's design removes.
   **Progress 2026-10-20** (not met): built whole (accept 1's first stage), mistune
   is one extension of 38 modules, built in 6.5 s locally (7.0 s as 38 extensions,
   the CI bound 10 s), its 1155 tests passing, 386 of 413 functions native,
   65.3M instructions per `mistune.html` pass (65.2M), 0.96x and 0.90x (best of 5,
   noisy host): the modules still reach each other's instances through CPython,
   which the next stages (native calls, then native classes across modules) change.
   **Progress 2026-10-21** (not met): with native calls between the modules (accept 1's
   second stage) 12 imported helpers of mistune are called natively, 386 of 413
   functions native, the build 5.9 s locally (CI bound 10 s), 65.48M instructions per
   pass against 65.42M without them: unchanged. Of the attribute operations its parse
   executes (line counts of the CPython run times each line's attribute uses), 38% are
   on `self` (the CPython-run parsers and renderers), 24% on `state` (the native
   `BlockState`/`InlineState`, reached through CPython from the other modules) and 39%
   on other objects. So native instances of another module's classes (next) alone
   cannot give 2x; the parsers themselves must become native classes too (constrained
   `TypeVar` in `Generic[...]`, a subscripted base from another module,
   `super().__init__()`).
   **Progress 2026-10-21** (not met): `BlockState` and `InlineState` are read natively
   in the modules that import them (accept 1's third stage, first part): 63.33M
   instructions per pass (64.51M). The parsers and renderers, whose classes CPython
   still creates, were to be next; an experiment decided against it (decision, with the
   guru advisor: measure one parser native before building what all of them need).
   Hand-editing mistune so that `BlockParser` compiles as a native class in one module
   (its base `Parser` merged in, its object class attributes moved to module globals,
   five shapes pycc does not compile yet rewritten) gives the same output and 63.45M
   instructions per pass against 63.48M with `BlockParser` created by CPython (CPython
   runs the sources in 57.19M): the parser classes are not where the time goes. The
   cost is spread over CPython objects the parse builds and reads (token dicts, `re`
   match objects, about 11.6M in `re` itself), so 2x on mistune (28.4M per pass) is out
   of reach of native classes; cross-module bases, object class attributes and
   constrained `TypeVar` bases are not built for it. Fixed on the way, each general:
   `super(C, obj)` in a method of a class CPython creates (394 of 413 functions native),
   and objects reaching `int | None`-style slots.
   **Progress 2026-10-21** (not met): mistune does not reach 2x and stays in
   `bench/package_check.py` and the CI wheel checks for its correctness value (0.8x-0.9x:
   63.5M against CPython's 57.2M instructions per pass, the cost in CPython objects --
   token dicts, `re` match objects, `re` itself about 11.6M per pass). The accept's
   package is chosen again by a rule fixed before measuring: the most-downloaded pure-
   Python template engine (pypistats, last month, 2026-10-20) that (a) has no C extension
   and no C-accelerated dependency, (b) renders by interpreting the template in its own
   code rather than generating Python source CPython compiles and runs (which pycc never
   sees), and (c) ships a self-contained test suite in its PyPI sdist. Jinja2 (534M) and
   Mako (181M) fail (a) and (b) (MarkupSafe's C speedups, generated Python); chevron
   (11.0M) fails (c) (no tests in the sdist; the repository's need the Mustache spec
   submodule); pystache 0.6.8 (3.89M, MIT) meets all three; genshi, cheetah3 and tempita
   are below 0.25M. Disclosure: before the rule was fixed, chevron was timed once
   compiled (1.00x, its `render` falling back) and pystache was profiled once under
   CPython; neither run chose the package. pystache is added to `bench/package_check.py`
   and the CI wheel checks (its 255 tests pass compiled; a class of it that its own test
   modules subclass, through two re-exports, now runs in CPython, the re-exports of any
   module being followed). Its own benchmark (`pystache/tests/benchmark.py`'s examples):
   0.58x (0.044 s against 0.076 s for 1000 rounds; 0.83M instructions per round against
   CPython's 0.50M), 105 of 111 functions native: as for pydantic and mistune, compiled
   code spends more on each CPython-object operation than the interpreter does
   (microbenchmarks of one operation on CPython objects, compiled against CPython: a
   method call 0.69x, a call with arguments 0.56x, an attribute store 0.58x, a dict
   display 0.76x; a `dict` read 1.9x, `isinstance` 16x), which is accept 1's amended
   lever.
   **Progress 2026-10-21** (not met), after accept 1's per-operation lever (attribute
   caches, in-place `str` compares, set displays built once, methods loaded before
   their arguments): pystache 0.77M instructions per round (CPython 0.50M, 0.65x by
   instructions; its benchmark 0.0821 s against 0.0935 s before the caches, CPython
   0.0566 s, 0.69x); mistune 58.0M per `mistune.html` pass (CPython 57.2M). Neither is
   near 2x, for accept 1's reason: their hot code is object-protocol work (pystache's
   context-stack lookups, `re` matches and dict displays), which compiled code now does
   at about the interpreter's own cost and no faster. Its six fallbacks (an `except` of
   another module's exception class, identity tests against a `NotFound` sentinel) are
   language gaps worth closing for nativeness, not a 2x lever. Not met: what would meet
   it is typed native representations of that hot state -- pystache's context stack
   and parsed template nodes, mistune's tokens, as compiled classes, lists and dicts
   with native `str` keys instead of CPython objects -- strict-mode work for a later
   milestone, as for accept 1.
5. **Windows parity and corpus:** native programs on Windows write CPython's text-mode
   stdout (`\r\n`, the console encoding), removing that DEVIATIONS row; corpus strict
   >= 193/200, `--non-strict` 200/200, 0 wrong, 0 fallback statements.

Progress:
- Accept 5 **not met** (2026-10-21): the Windows half holds, the corpus half stops at
  191/200 strict. Windows: a native program writes CPython 3.13's text-mode stdout and
  stderr -- `\r\n`; UTF-8 on a console; otherwise `PYTHONIOENCODING`, UTF-8 mode, or the
  ANSI code page with CPython's `cp1252` table, `surrogateescape`/`backslashreplace`,
  `UnicodeEncodeError` on an unencodable character -- and reads a redirected stdin in
  the same codec (`crates/pycc_rt/src/text_stdio.rs`; `tests/text_stdio.rs` compares raw
  bytes with CPython on the Windows runner, ANSI code page and five `PYTHONIOENCODING`
  values included: CI run 39543283114 on 5bc2d2c, all four jobs green). The DEVIATIONS
  row is replaced by a narrow one
  (rare codecs). Tests compare captured output as line content (`pycc_scratch::lf`).
  Corpus: strict 190 -> 191 (`[name, index]` displays are lists of a fixed shape,
  `Ty::ShapedList`, which 070 needs); `--non-strict` 200/200, 0 wrong, 0 of 1191
  statements in CPython. Design, agreed with the guru advisor: strict mode compiles a
  union only where every operation on it is valid for every arm (as `int | float`
  is), so the remaining nine stay refused -- each is a program a type checker rejects:
  a `str` element replaced by an `int` and then summed (130) or accumulated (049), a
  `list | set` name whose list arm reaches `|=` (200), `[0] + [None] * n` receiving
  `str` elements and then joined (123), and an `int | None` used as an `int` with no
  `None` test (016, 056, 057's `group[-1][0] - left`, 100, 184). The
  193 bar needs two of those to compile, which would be strict typing weakened; the
  item stays open on that.
- Accept 2 **Met 2026-10-20** (CI run 39500973184 on 926c692, all four jobs green):
  pydantic 2.13.5 1029/1249 functions native (`bench/pydantic_check.py`), its suite
  unchanged; methods of CPython-run classes call their compiled bodies with no forwarder
  frame (`src/ext/module/forward.c`, with accept 3). History: the `...` literal (a bare `...` statement is a no-op; the value
  is the `Ellipsis` object), `hash()` of a CPython object and function-local imports of
  CPython modules compile natively, and in non-strict code a nested `def` or `lambda`
  used as a value runs in CPython alone, its body compiled and its enclosing function
  compiled (`tests/v08_constructs.rs`); pydantic 2.13.5: 978 of 1249 functions native
  (`bench/pydantic_check.py`, from 809; 1021 before typing's `Union`/`Any`/`Annotated`
  became compiled names, whose module-level value uses now fall back), its suite failing
  the same 20 tests. Remaining: those value uses. The call boundary's forwarder frame is
  gone (with accept 3): once a class statement CPython runs has run, each method's
  forwarder calls its compiled body through a C vectorcall trampoline
  (`src/ext/module/forward.c`), 7 ns less per call of a method with exact positional
  arguments (41 vs 48 ns); a call with keywords or defaults binds in C at the forwarder's
  cost.
- Accept 3 **Met 2026-10-18** (CI run 39109040009 on fa82859, all four jobs green; narrower
  DEVIATIONS rows remain: empty `f_locals` of a compiled frame, no `settrace`/`setprofile`
  events, statement-wide carets for some operations, frameless functions on CPython other
  than 3.13): on CPython 3.13 compiled functions run in real
  CPython frames (`tests/v08_frames.rs`: `sys._getframe` chains through compiled and
  Python callers, `inspect.stack()`, tracebacks with CPython's lines and carets, a
  traceback formatted in a compiled handler, `warnings` `stacklevel`, module globals
  rebound from Python, published private functions, constant memory; a native program's
  traceback reads its source at run time). Design, agreed with the guru advisor: the
  `abi3` shim mirrors 3.13's `_PyInterpreterFrame`, `PyFrameObject`, `PyCodeObject` and
  `PyFunctionObject` layouts and uses them only after an import-time probe; each function
  gets a synthetic code object (one `NOP` per source position); a function that can run
  CPython code links its frame onto the thread's chain, one that only computes natively
  gets its traceback entry when the exception reaches CPython; a rebindable global is
  re-read when the module dict's version tag changed. Other CPython versions keep
  frameless compiled functions (docs/DEVIATIONS.md). pydantic 2.13.5's suite: 0 failures
  (was 20 more than against its sources; 5762 passed). Cost: a framed call about 10 ns
  more, a frameless one nothing; `bench/run.py`'s native programs run the same
  instructions or fewer; `bench/package_check.py --instructions` within bounds;
  `bench/pydantic_check.py --time` (15 runs, CPython/compiled median ratio, before ->
  after): import 0.74x -> 0.80x, define 0.67x -> 0.66x, schema 0.67x -> 0.66x,
  round-trips 0.94x -> 0.95x.

## v0.9 — Typed hot data, native end to end (released 2026-10-21)

v0.8 showed where the speed is: code over typed native data runs many times faster than
CPython (diff-match-patch ~12x, pypng ~12x), while dynamic code over CPython objects stays
at about CPython's speed however it is compiled (pydantic define/schema ~0.8x, mistune ~1x,
pystache ~0.7x; the analysis is in v0.8 accepts 1 and 4). v0.9 makes libraries' hot state
typed and native: their own classes, lists and dicts become native values that compiled
code of every module of the package reads directly.

Released as 0.9.0 with accepts 1 (in its stated exact form) and 5 met; accepts 2, 3 and 4
are not met (pystache, mistune and pydantic stay below their bars; the evidence is in each
accept's note below).

Accept:
1. **Native hot state in `--non-strict`:** instances of a package's own classes, and lists
   and `str`-keyed dicts built by its compiled code, are native when their element types
   infer (with a guarded object fallback where they escape to CPython), including classes
   whose bases come from another module of the package, `Generic[...]` bases with plain or
   constrained `TypeVar`s, and `lambda`s in class bodies (deferred from v0.8).
   **Progress 2026-10-21** (not met), step 1 -- the general reasons a library class went
   to CPython. Design, with the guru advisor: make the classes native first, then type-
   guarded native dispatch on object receivers whose run-time class is a compiled class
   (keyed by the carrier type pointer, filled lazily per site, candidates with object
   parameters first), rather than unions of instance types. Done: a list or dict slot
   holding objects is a CPython list or dict (it was `list[object]` while `[]` built a
   CPython list, a mismatch that sent the class to CPython); an object-typed attribute
   called like a method (`self.escape(s)`); `obj is instance` (a sentinel) through the
   instance's carrier; a helper called with instances of several classes, or returning an
   instance on one path and an object on another, takes and returns objects; a fresh
   display passed where a parameter is an object; a parameter only `None` reaches is an
   object. Correctness for escaping instances: a private class is published in
   non-strict mode with its methods (also private ones taking instances or `None`), so
   Python code holding a node calls `node.render(...)`; a carrier answers `__dict__`
   (a snapshot); a class defining the descriptor protocol (`__get__`, `__set__`,
   `__delete__`, `__set_name__`) runs in CPython, since a carrier takes no part in it.
   A module-attribute cache bug: a dict key equal to but not identical with the name
   read (what a non-strict module's compiled globals store) never filled the cache, so
   `defaults.X` from another module was a full lookup with two failed probes every time
   (pystache's `Renderer()`: 35k -> 6.9k instructions). pystache: `RenderEngine`,
   `Loader`, `ParsedTemplate` and every node class native (105 -> 108 of 111 functions);
   `ContextStack` (`*args`), `_Parser` (a class attribute default of an instance
   attribute), `Renderer` (bound methods as values) and the exception classes (a base
   from another module) still run in CPython. pydantic unchanged (1029 of 1249, its suite
   0 failures), mistune unchanged.
   Step 2 -- `*args`/`**kwargs` of objects: a constructor `def __init__(self, a, *items)`
   is bound by `Py_tp_init` as CPython binds it (keywords, CPython's messages); a
   `@staticmethod` or module function with unannotated `*args`/`**kwargs` is published
   through a wrapper of its declared signature (a static method's row is `METH_STATIC`);
   `except` naming an imported exception class; dict/set displays inside an object
   display; `obj.__class__` and `obj.__dict__` of a compiled instance. pystache's
   `ContextStack` native.
   Step 3 -- native closures with object parameters: a nested function with unannotated
   parameters used as a value is lifted to a native closure over its stable captures
   (parameters and return `object`, so it crosses into CPython as a callable). pystache's
   `get_unicode` (`map(get_unicode, ...)`), `resolve_context` and the other returned
   closures native: Python frames per round 127 -> 52; steps 2 and 3 together 783k ->
   611k instructions per round in steady state (rounds 50..250; an earlier 488k was an
   artefact of a baseline run that compiled the tests' bytecode).
   Step 4 (with the guru advisor: measure steady state; shrink hot code; remove the
   remaining frames; devirtualize last): building with LLVM's `O2`/`Os` instead of `O3`
   or without inline reference counting changed neither the artifact's size nor its wall
   time beyond noise -- the hot code is the runtime's object-call paths, not the
   optimizer's output. What moved: `html.escape`/`re.escape` computed natively at call
   sites their imports mark (identity-guarded, CPython's results), module-level lambda
   aliases published compiled, and two-way set associative attribute caches (two keys
   hashing to one entry evicted each other on every read, refilling by an MRO walk:
   up to 57k instructions per round, depending on layout). 611k -> 582k instructions,
   Python frames 52 -> 40 per round.
   Step 5 (design with the guru advisor: CPython makes each published class): a
   published compiled class is created when its statement runs, by its metaclass, from
   the statement's real bases and a compiled template (methods, carrier layout, slots;
   every template derives from one root so multiple inheritance keeps one layout), and
   is entered as its instances' carrier type. `Generic[...]` bases with plain,
   constrained or bounded type variables (non-strict) and `lambda`s in class bodies
   (methods, `staticmethod(lambda ...)`, `classmethod(lambda ...)`) are native; a
   compiled subclass is a CPython subclass of its compiled base. Still open in
   accept 1: bases from another module of the package (the template must extend the
   base's layout) and exception classes over such bases, Python subclasses of compiled
   classes.
   Step 6 (guru ruling: virtual dispatch for the override set the build knows, realized
   as dispatch to D-254's receiver-exact bodies): a method call whose receiver's class
   has a compiled subclass resolving the method to another body calls one dispatcher
   per (class, method) testing the receiver's run-time class. Fixes a pre-existing
   divergence (a subclass instance passed where its base is annotated ran the base's
   bodies) and is the mechanism cross-module bases build on.
   Step 7a (guru: Python-run subclasses first, as the fallback arm of native
   cross-module subclasses): Python subclasses of published compiled classes. A
   dispatcher's first arm tests the instance's strong-carrier flag inline and runs a
   Python override through CPython; the instance keeps its subclass object alive, the
   cycle collected through the template's `tp_traverse` (templates are GC types whose
   own objects are untracked at creation, so compiled classes' objects cost the
   collector nothing). A compiled base a sibling module subclasses stays native, its
   subclass a CPython class with compiled methods.
   Step 8: class-level `None` defaults that instances shadow become instance slots
   initialized by `__init__` (the published class keeps the default), which makes
   pystache's `TemplateSpec` native with its sibling-module subclasses over it.
   Accept 1's cross-module clause, in its exact form: a compiled class whose base is a
   compiled class of another module of the package compiles with the base native and
   the subclass a CPython class over the base's layout plus a `__dict__`, its methods
   compiled as lifted bodies, with CPython's real MRO (pystache's `TemplateSpec`
   subclasses exercise it in its suite). Native-native subclassing across modules is
   deferred (guru ruling): no cross-module relation in pystache, mistune or pydantic
   qualifies -- pystache's other one is an exception class, mistune's bases are
   `Generic[ST]` with private members and `getattr` dispatch, pydantic's handlers are
   private-member heavy. Also deferred: exception classes over package bases (PEP 697
   layouts) and bases that publish only part of themselves.
   **Met 2026-10-21 in this exact form:** CI run 39652820996 on 69eb20c, green on all
   four jobs (`tests/v09_native_state.rs`: dispatchers, Python subclasses, instance
   defaults; pystache's suite with `TemplateSpec` native).
2. **Template engine >= 2x:** pystache 0.6.8 (chosen by the v0.8 accept 4 rule) runs its
   own benchmark >= 2x faster than CPython (median of interleaved runs) and passes its suite.
   **Progress 2026-10-21** (not met): 0.61x -> 0.69x after accept 1's step 1 (0.089 s
   against 0.062 s, 3 interleaved runs). What any compile of pystache leaves to CPython,
   measured (callgrind, CPython on the sources, per round of the benchmark's two examples,
   of 505K instructions): the regex searches 83K (the fixed part); `match.start`/`end`
   and `groupdict` 40K; `_compile_template_re` per parse (`re.escape` twice, `%`
   formatting, `re.compile`'s cache hit hashing a ~300-character pattern) 35K; 13
   `html.escape` calls 42K -- `html.escape` and `re.escape` on a `str` can be compiled
   natively with CPython's results, so the fixed floor is ~110-130K and 2x needs the
   rest under ~125K. The compiled tree spends 650K: it removed 134K of interpreter
   work but pays as much in C-API calls (object method calls through CPython, Python
   frames for the lifted closures and lambdas, boxing), so the next steps remove those:
   native dispatch on compiled instances, native closures, native `html.escape`.
   After step 2 (`ContextStack` native): 0.62x by wall time this run (0.100 s against
   0.062 s, 9 interleaved runs; the machine was noisier than for step 1), 734k against
   505k instructions per round. After step 3 (native closures): 0.74x (0.081 s against
   0.060 s, 15 interleaved runs) at 611k instructions per round against CPython's 505k.
   Steady state with this machine's cache geometry (callgrind, rounds 50..250): 21.0k
   I1 misses per round against CPython's 8.2k, 10.5k mispredicted branches against
   6.8k; the misses are spread over the runtime's generic object-call paths and
   CPython's C API they enter. After step 4: 0.75x (0.080 s against 0.060 s, 11 runs),
   582k instructions per round. The largest remaining cost is calls on objects whose
   run-time class is a compiled class (`node.render(engine, context)` on a node read
   from a list of objects): ~60k instructions per round in the runtime's call path plus
   CPython's -- the guarded devirtualization planned next.
   **Decision 2026-10-21: not met for v0.9; the floor, measured.** After accept 1's
   steps 5-8 and the per-call-site method cache (the last lever, guru ruling): 586k
   instructions per round against CPython's 505k (0.86x by instructions; 0.73x-0.94x
   by wall time on this loaded machine, interleaved medians). The benchmark round's
   work that stays CPython's whatever compiles pystache (callgrind on CPython running
   the sources, per round): the regex searches 83k; `match.start`/`end`/`groupdict`
   40k; `_compile_template_re` per parse 35k (`%` formatting and `re.compile`'s cache
   lookup of a ~300-character pattern; its `re.escape` calls are native now) -- a
   floor of ~158k. `html.escape` (42k in CPython) is native and no longer part of it.
   2x needs at most ~252k per round, so everything compiled code itself spends would
   have to fit in ~94k; it spends ~428k, mostly crossing the CPython boundary rather
   than computing: method calls on objects (the per-site cache took ~5k of them),
   `str` copies at each crossing, the template context's lookups through CPython
   dicts and `getattr`, and the remaining Python frames of lifted closures. Closing
   that gap needs the template's data (context stacks, parsed nodes, the rendered
   parts) to stay native end to end, a v0.10-scale change; the accept stays open with
   this evidence.
3. **Markdown >= 1.5x:** mistune 3.3.4 renders `benchmark/cases` >= 1.5x faster than
   CPython and passes its suite.
   **Measured 2026-10-21, not met for v0.9:** its suite passes compiled; `mistune.html`
   of every `benchmark/cases` document x20 runs at 1.05x (0.124 s against 0.131 s) and
   the benchmark's full parser with every plugin at 0.84x (0.141 s against 0.119 s),
   medians of 11 interleaved runs on main at 69eb20c (`bench/package_check.py mistune
   --time --runs 11`). Its hot classes stay CPython classes: the parsers derive from
   `Parser(Generic[ST])` with private members and its renderers dispatch through
   `getattr`, which accept 1's work does not make native (see accept 1's note).
   - Amendment (2026-10-21, release re-measurement, 15 interleaved runs, twice):
     `mistune.html` 1.00x and 0.86x, the full parser 0.97x and 0.93x -- about CPython's
     speed, within this shared machine's noise. Still not met.
4. **pydantic no slower:** every phase of `bench/pydantic_check.py --time` (import, define,
   schema, round-trips) >= 1.0x CPython's median; its suite fails no test that passes
   against its sources.
   **Measured 2026-10-21, not met for v0.9:** the suite fails no test that passes against
   the sources, but the phases run at import 0.85x, define 0.72x, schema 0.74x,
   round-trips 0.97x (medians of 11 interleaved runs on main at 69eb20c,
   `bench/pydantic_check.py --time --runs 11`); 1036 of 1249 functions are native, the
   same count as before v0.9's work, which targeted pystache's and mistune's classes.
   - Amendment (2026-10-21, release re-measurement): the import 0.85x above was noise,
     not a regression. Medians of 21 interleaved runs, main (7784571) and v0.8.0 built and
     measured back to back on the same machine, twice: main import 0.98x / 0.91x, define
     0.80x / 0.78x, schema 0.83x / 0.77x, round-trips 0.94x / 0.94x; v0.8.0 import 0.92x /
     0.97x, define 0.74x / 0.77x, schema 0.79x / 0.75x, round-trips 0.95x / 0.90x.
     Instructions of `import pydantic; from pydantic import BaseModel` against the built
     trees (callgrind): 375.6M on main, 372.1M on v0.8.0 (+0.9%). Still not met.
5. **Strict refuses exactly what a type checker refuses:** a CI check runs `mypy --strict`
   over the corpus; every strict refusal is a program mypy also rejects (or is listed in
   DEVIATIONS with a reason), and `--non-strict` stays 200/200, 0 wrong, 0 fallback
   statements. Issues #8 and #9 are fixed.
   **Met 2026-10-21:** CI run 39585350033 (all four jobs green), step "Strict refusals
   against mypy --strict" (`bench/mypy_parity.py`, mypy 2.3.1 pinned by hash in
   `.github/mypy-requirements.txt`): pycc strict refuses 9 of the 200 programs and mypy
   rejects 8 of them (016, 056, 057, 100, 123, 130, 184, 200); the ninth, 049, passes mypy
   only through `Any` (it rebinds the builtin `input`) and is listed in DEVIATIONS ("Strict
   refusals mypy accepts"). 113 programs mypy rejects and pycc accepts, almost all for
   annotations pycc infers. "Corpus (non-strict)": 200/200, 0 wrong, 0/1191 top-level
   statements in CPython. Issues #8 and #9: commit fec07f2 (a non-strict native build
   publishes its classes in `__main__`), `tests/issue_8_9_native_classes.rs`.

## v0.10 — Native dicts, exact errors, trustworthy numbers

v0.9 left libraries' hot state in CPython dicts (pystache's context stack, mistune's
tokens) and its speed numbers came from a shared VM. v0.10 makes `str`-keyed dicts native,
closes the error-text gaps, finishes the deferred class work, and moves published timings
to a reproducible CI benchmark job.

Accept:
1. **Exact error texts:** issue #10 is fixed (argument-binding `TypeError`s and
   type-attribute `AttributeError`s match CPython 3.13 word for word), and the differential
   fuzzer generates calls with missing/extra/keyword arguments and missing attributes on
   classes and instances, comparing exception messages; 0 findings in a 30-minute run.
   **Met 2026-10-29:** a published function, method or constructor (`--ext`, a
   `--non-strict` native build's `__main__`) raises CPython's `missing ... argument`,
   `takes N positional arguments but M were given` and `Did you mean 'x'?` texts, and
   compiled classes and instances CPython's `AttributeError`s
   (`tests/issue_10_error_texts.rs`). The fuzzer's `call_errors` block calls with
   missing, extra, unknown, misspelled and repeated arguments and reads missing
   attributes; the nightly-fuzz workflow dispatched on the change (run 40881731071,
   e34d5cf) compared 2996 programs in 1801 s (376 with the block) with 0 findings, after
   the block's first findings were fixed with fixtures in `tests/fuzz_regressions.rs`.
   CI run 40881711412 green on all four jobs at e34d5cf. DEVIATIONS lists what stays
   different: an unbound method called with no or a foreign receiver, an instance field
   read through its class.
2. **Native `str`-keyed dicts:** in `--non-strict`, dicts with `str` keys built or received
   by compiled code are native when their value types infer, with a guarded CPython-dict
   fallback when they escape or arrive from CPython code; iteration order, views, mutation
   during iteration errors, `==`, `repr`, pickling and identity match CPython.
   **Realization (guru ruling, 2026-10-29):** escape is decided per allocation site at
   build time (the non-strict widening that already makes a typed dict meeting an object a
   CPython dict from creation); a dict that may escape is a CPython dict from creation,
   and every CPython dict -- built escaping, or received -- is accessed through direct
   dict-API calls under an exact-dict guard, so identity, views, pickling and iteration
   errors are CPython's by construction for every dict CPython can see. No run-time
   materialize-and-forward. What stays to do: a non-escaping `str`-keyed dict whose values
   are objects (`{"type": "text", "raw": s}`) is a native dict too -- open: neither
   package has such a dict on its hot path while mistune's renderers are CPython classes
   (accept 3's table).
   **Progress 2026-10-29:** step 1, the exact-dict guard on `d[k]`, `k in d` and
   `d[k] = v` of CPython dicts, and literals crossing as CPython's interned strings
   (`tests/v10_dicts.rs`): 275 -> 179 instructions for a compiled `d['raw']` on a
   received dict. pystache 584k -> 580k per round, mistune 58.7M -> 57.7M
   per pass (CPython 505k and 57.0M, `PYTHONHASHSEED=0`, each count after a discarded
   warm-up run that writes the trees' `.pyc` files): dicts are not where either package
   spends (accept 3's note).
3. **Template and Markdown >= 1.2x:** pystache 0.6.8 and mistune 3.3.4 each run their own
   benchmark >= 1.2x faster than CPython (median of the CI benchmark job, accept 4), with
   their suites passing.
   **Profile 2026-10-29** (callgrind, main 97e5e9f, per round, `PYTHONHASHSEED=0`):
   pystache 580k against CPython's 510k, of which parsing (`parse()` alone) 238k against
   214k and rendering with the call plumbing 342k against 295k; 1.2x needs 424k in all.
   The plumbing of `render(**kwargs)` -> `Renderer.render(*context, **kwargs)` ->
   `_render_string` -> `_render_final` -> `ContextStack.create` and the per-render
   `Renderer`/`RenderEngine` construction is ~110k: `Renderer` is a CPython class
   because the package's tests subclass it and its members (private methods, defaults,
   `*args`/`**kwargs`) are not published. mistune 58.7M against 57.2M per pass:
   CPython's interpreter is ~22M of its time and compiled code's boundary costs about as
   much (calls and method caches ~4M, attribute lookups 0.7M, `str` crossings ~1.6M,
   dict operations ~1.1M); its renderers are CPython classes, so its tokens cross into
   CPython at once.
   **Progress 2026-10-29:** a nested function used as a value whose captures are all
   bound before its `def` (a parameter normalized first: pystache's `get_unicode`,
   `_render_string`'s callbacks) is a native closure, and `sorted(key=)` of one calls
   its lifted function in a comprehension (`tests/v10_closures.rs`). Its `map`/`filter`
   stay CPython's lazy builtins over the compiled function: an eager rewrite of them
   (564k) could not keep CPython's laziness, captured-state reads and `StopIteration`.
   pystache 582k per round (580k before; CPython 505k); mistune unchanged (57.7M).
   **Decision 2026-10-29: not met for v0.10; what is left, estimated.** The bar is ~421k
   instructions per round for pystache (CPython 505k / 1.2; now 582k) and ~47.5M per pass
   for mistune (57.0M / 1.2; now 57.7M). The remaining levers, each measured where it
   costs now (callgrind, `PYTHONHASHSEED=0`) and its saving estimated:

   | Package | Lever | Costs now | Saving |
   |---|---|---|---|
   | pystache | `Renderer` native: publish private, defaulted and `*args`/`**kwargs` methods to the package's Python subclass, box bound methods (`self._to_unicode_hard`) into CPython calls, lambda values | its methods and the `render(**kwargs)` plumbing ~120k | 60-80k |
   | pystache | `_Parser` native (private and non-`None` literal instance defaults) | `parse()` 238k (CPython 214k, ~158k of it CPython's own) | 15-25k |
   | pystache | engine callbacks and context lookups as native calls once `Renderer` is native | node renders, `fetch_string`, context chain ~120k | 20-30k |
   | pystache | per-site method cache for dict-bearing CPython classes, calling a forwarder's compiled entry directly | `Loader` and the other CPython classes' calls | 5-10k |
   | mistune | the same per-site cache and direct forwarder entry | method calls, caches, loads ~4.6M | 1.5-2M |
   | mistune | renderers and parsers native (`Generic[ST]` templates, a name-to-method table for `getattr(self, name)` dispatch, private members) | attribute lookups 0.7M, forwarder calls | 4-6M |
   | mistune | tokens as native dicts (accept 2's open piece), after the renderers | dict operations ~1.3M | ~0.7M |
   | mistune | fewer `str` crossings | ~1.6M | ~0.8M |

   Summed: pystache 437k-482k (1.05x-1.16x), mistune 48.2M-50.7M (1.12x-1.18x); neither
   reaches 1.2x even at the optimistic end, and the largest levers are multi-feature
   projects. The accept stays open with this evidence.
4. **Trustworthy numbers:** a CI benchmark workflow runs `bench/run.py`, the package
   `--time` checks and `bench/pydantic_check.py --time` on a GitHub-hosted runner with
   interleaved fresh-process runs, and publishes median and spread as an artifact; every
   speed number in README, CHANGELOG and blog posts cites such a run. Instruction-count
   bounds stay the regression gate.
   **Met 2026-10-29:** `.github/workflows/bench.yml` (`workflow_dispatch`, release tags,
   weekly; `contents: read`) runs on ubuntu-24.04 and ubuntu-24.04-arm: `bench/run.py`
   (CPython, pycc-next and the Rust twin as fresh processes, interleaved per round), the
   five packages' `--time` checks (mistune and pystache among them; one fresh process per
   tree and round, alternating, after a discarded warm-up) and `bench/pydantic_check.py
   --time`, and publishes medians with min-max (quartiles and every sample in the JSON),
   the runner's CPU model and the run count as the job summary and the `bench-<os>`
   artifact (`bench/publish.py`). Run 40866079503 at 3f9e371 green on both runners
   (x86_64: Xeon Platinum 8370C, 11 runs; pydantic 15). README's speed tables and package
   numbers cite it; the `bench`, `release` and `blog-post` skills require every speed
   number published from now on (README, CHANGELOG, release notes, blog posts) to cite
   such a run. CHANGELOG sections and blog posts of released versions stay as published:
   dated records measured before the workflow existed. That run, for accept 3's baseline:
   pystache's benchmark 0.63x, mistune `mistune.html` 0.83x and the full parser 0.79x;
   pydantic import 0.93x, define 0.79x, schema 0.82x, round-trips 0.91x.
5. **Deferred class work:** compiled subclasses of compiled classes from another module of
   the package are native (layout-pointer dispatch across modules, no CPython class in
   between), and compiled exception classes can derive from package exception bases
   (PEP 697); each with oracle tests vs CPython.
   **Progress 2026-10-29** -- the cross-module half (design, guru ruling: the plan gives
   the base module the *set* of overridden names, the subclass's own class record holds
   the *targets*): every class has a record, one named descriptor global its instances
   point at (a run-time class test is a pointer compare);
   a member compiles `class Square(Shape)` over the sibling class standing for
   `base.Shape` (its slots and, from `base`'s interface, its `__init__` and each method's
   index in `Shape`'s method table), and `Square`'s record carries a vtable of its
   resolved methods (null for an override of another signature) whose `base` is
   `base`'s record. `base`, told by a syntactic scan of the package which names its
   classes' subclasses elsewhere define, gives each such (class, method) a dispatcher with
   a foreign arm: a receiver whose record carries a vtable runs its entry, or the member
   through CPython when the entry is null; a call on a value typed with the sibling class
   runs that dispatcher (`tests/v010_subclasses.rs`). Open: a subclass of such a subclass
   in a third module (it runs as a Python subclass).
   **Progress 2026-10-29** -- the exception half, single module (design, guru ruling:
   CPython makes the class from its statement, the template first among its bases): an
   exception class with members of its own keeps `lift`'s split (CPython runs the
   statement, its methods compiled), and the attributes its methods store from
   parameters go in a native layout -- a hidden compiled class declares it, the
   statement's bases pass through `__pycc_exc_bases__`, which puts a PEP 697 template
   over the exception base first (its objects' payload holds the native instance, bound
   unassigned on first native use; it inherits the base's `tp_new`, so `__new__` resolves
   along the MRO as for the source class; its descriptors, `__reduce__`, deallocator,
   traverse and clear chain to the base's; a name any base binds at class level, such as
   `OSError.errno`, stays CPython's), and
   each lifted method takes the object's native view as a second receiver
   (`tests/v010_exceptions.rs`). Across the modules of a package built whole, a
   syntactic scan tells which bases are another module's exception classes and
   whether their chain may have a layout: over one without, a class gets a layout of
   its own (pystache's `KeyNotFoundError(PystacheError)` shape); over a layout class it
   keeps that class's, its own attributes in `__dict__` (`tests/fixtures/v010_exc_pkg`).
   Open: a layout extending another module's.

## v0.11 — Imports faster than CPython

v0.10's CI benchmark job put pydantic's import at 0.93x CPython (455.7M instructions vs
442.9M); the rest is a 10.5 MB extension to load, ~8.8M instructions of libpython object
creation (functions, classes, forwarders, signatures) and ~296 fallback chunks run at
import. v0.11 makes importing compiled packages at least as fast as importing their sources,
then faster, without changing what Python code can observe.

Accept:
1. **pydantic import >= 1.0x**, then **>= 1.2x** as a stretch: median of the CI benchmark
   job (`bench.yml`) for `import pydantic; from pydantic import BaseModel`, compiled tree vs
   CPython from `.pyc`, same pydantic-core wheel; its suite fails no test that passes
   against its sources.
2. **Smaller, faster-loading extensions:** whole-package builds use LTO across the
   package's modules, identical-code folding and cold sections for error and fallback
   paths; the pydantic extension's size and load-time relocations are measured and reduced,
   with no runtime instruction-count regression on the CI bounds.
3. **Less work at import:** fewer fallback chunks executed at import (functions and classes
   created natively where possible), and lazy creation of module-level objects only where
   it is unobservable to Python code (module `__dict__` contents, `dir()`, `inspect`,
   pickling, import side-effect order and exceptions match CPython); every laziness rule is
   documented with its observability argument and covered by oracle tests.
4. **General, not pydantic-only:** mistune's and pystache's import times are measured in
   the CI benchmark job and are no slower than CPython's; no package check regresses.
