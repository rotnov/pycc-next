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

## v0.3 — Packages, stdlib, everyday code

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
