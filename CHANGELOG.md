# Changelog

All notable changes to pycc-next. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- `bench/package_check.py --time` times `import mistune` and `import pystache`, one import
  per fresh process against the CPython tree's `.pyc` files, so the CI benchmark job
  publishes both (v0.11 accept 4); a workload CPython runs in under 0.1 s prints and
  publishes in ms.

### Changed
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

## [0.10.0] - 2026-10-30

### Added
- The `bench` CI workflow (`.github/workflows/bench.yml`; manual, on release tags and
  weekly, on ubuntu-24.04 and ubuntu-24.04-arm): `bench/run.py`, the real packages'
  `--time` checks and `bench/pydantic_check.py --time` with interleaved fresh-process
  runs, published as medians with min-max, the runner's CPU and the run count (job
  summary, `bench-<os>` artifact). Published speed numbers cite a run of it (v0.10
  accept 4).
- Cython 3.3.0 and mypyc (mypy 2.3.1) as benchmark competitors (`bench/competitors.py`,
  pinned in `.github/competitor-requirements.txt`): `bench/run.py` and
  `package_check.py`/`pydantic_check.py --time --competitors` build the same unmodified
  sources with each and time what builds and passes; the published result has Cython and
  mypyc columns, pycc vs each, the Cython mode, and every build that failed with why.
  CPython stays the correctness oracle.
- The differential fuzzer's `call_errors` block: calls with missing, extra, unknown,
  misspelled and repeated arguments, and missing attributes of classes and instances.
- `--non-strict`: a function's `str`-keyed dict display holding CPython objects
  (`{"first": None, "n": 0}`) is a native dict when the dict never escapes the function
  -- it is only read, stored into, tested for a key, measured, iterated or tested for
  truth; returned, passed, stored elsewhere, compared or printed, it stays CPython's
  `dict`, so identity, views, pickling and `repr` are CPython's wherever CPython can see
  it (v0.10 accept 2). Equal identifier-like `str` literals share one native object, and
  native `str`-keyed dicts read a key's cached hash in place.

- Exception classes with members of their own can derive from another module's
  exception class in a package built whole (v0.10 accept 5): CPython makes each from its
  statement and its methods are compiled, so `args`, `str`, tracebacks, pickling and
  class-statement hooks are CPython's, and functions raising and catching them stay
  compiled. A native attribute layout for them (PEP 697) was measured 2.5x slower on raise
  and catch and 1.5x slower on attribute reads than this, and not shipped.

- Instances of published compiled classes pickle (v0.10), as `object.__reduce_ex__` pickles
  an object: `copyreg.__newobj__` with `__getnewargs_ex__` / `__getnewargs__`'s arguments and
  the state `object.__getstate__` gives the source class (each assigned compiled attribute
  as stored, a subclass's `__dict__` and `__slots__`), restored as pickle restores it, at
  every protocol (0 and 1 as `copyreg._reduce_ex`), with `copy.deepcopy`, shared
  references, nested instances and Python subclasses and their hooks (`__reduce__`,
  `__reduce_ex__`, `__getstate__`, `__setstate__`, `__new__`, `__getattribute__`,
  `__setattr__`, `__slots__`, `copyreg.dispatch_table`); one with an attribute Python code
  cannot see refuses with a `TypeError` naming it.

- Compiled subclasses of compiled classes of another module of a package built whole are
  native (v0.10 accept 5): `class Square(Shape)` with `Shape` imported from `.base` is
  compiled over `base`'s instance layout, and `base`'s compiled methods run `Square`'s
  overrides. Every class has a *record*, one named descriptor global its instances point
  at; a run-time class test is a pointer compare, and a subclass compiled over another
  module's class carries a vtable that module's per-(class, method) dispatchers read.

### Changed
- `C.__new__(C)` of a published compiled class makes an object whose attributes are all
  unassigned (reading one raises CPython's `AttributeError`) instead of an uninitialized
  carrier that compiled methods, setters and argument checks refused with a `TypeError`;
  `copy.copy` of a Python subclass's object that hooks the copy protocol goes through it.
- `bench/run.py` reports the median and min-max of interleaved fresh-process runs
  (default 11) instead of the best of 7; `bench/package_check.py --time` times each run
  in a fresh process, alternating CPython and the compiled tree; all three timing
  scripts write `--json`.
- `--non-strict`: compiled code reads, tests and stores into an exact CPython `dict`
  through the dict API (`d[k]`, `k in d`, `d[k] = v`), a subclass still through its own
  methods (`__missing__`, overridden `__getitem__`), with `dict`'s own `KeyError`s; a
  `str` literal CPython would intern as a code constant crosses as the interned string,
  so `'raw' is key` holds as in CPython and lookups keyed by it compare by identity
  (v0.10 accept 2). A compiled `d['raw']` on a received dict: 275 -> 179 instructions.
- A nested function used as a value is a native closure also when a variable it reads
  is rebound before its `def` and never after (`template = str(template)`, then
  `def piece(item): ...`), in strict and non-strict mode; it ran its `def` in CPython
  (or was `C0001` in strict mode). `sorted(xs, key=f)` of such a closure calls it
  directly in a comprehension instead of passing a function object to CPython's
  builtin; `map`/`filter` of one run CPython's lazy builtins over the compiled
  function.
- A whole-package build takes less CPU and finishes sooner (mistune, 38 modules, 4 jobs:
  5.25 s -> 4.37 s, 18.8 s -> 16.7 s of CPU, medians of 5 interleaved local builds; the
  extension is byte-identical). Each member build uses its share of the cores
  (`--member-threads`): with every core already building a member it no longer runs the
  frontend's speculative attempt without specializations beside the specialized one,
  which ran for every module and was discarded whenever the specialized attempt passed;
  it runs only when a failure needs it. The members compile against the package build's
  probe of the interpreter (`--ext-probe`) instead of each starting it again; they start
  by the longest chain of work that waits on them, estimated from the sources' sizes, so
  a large module no longer starts last and finishes alone; and the runtime unit and the
  package unit compile while the members build instead of after them. The script that
  precompiles a module's CPython source reads `marshal` data instead of JSON and imports
  `ast` only for a set display, halving its start-up.

### Fixed
- `key=` functions are called when CPython calls them.
  - **`sorted(xs, key=f)`:** keys a copy of its whole input. An input that raises part
    way raises before the first key call, and a key that changes the input (directly
    or through a property getter) does not change what is sorted.
  - **`min`/`max`:** each key is compared as soon as it is computed, as in CPython.
    Instance keys are compared through the class's `__lt__`/`__gt__` while the pairs
    are pulled from a generator, so key calls, comparisons and their exceptions
    interleave as CPython's do (a `StopIteration` a key raises propagates as it is,
    not as PEP 479's `RuntimeError`). Keys of numbers, `str` or tuples of them
    (immutable values whose comparison runs no code) are all computed first, which
    nothing can tell apart. Any other key (a list, a CPython object) is `C0001` in
    strict mode and runs in CPython in non-strict mode.
  - **`.sort` and `reverse=`:** `.sort` with a nested-function key runs as written, and
    with a key only a literal `reverse=` is rewritten (CPython evaluates every argument
    before the first key call).
- `super().__init__(...)` in an exception class with its own `__init__`, reaching the
  builtin exception's constructor, compiled to a call that failed at run time with
  `NameError: name 'Exception.__init__' is not defined`; it is a compile-time `C0001` now
  (in `--non-strict` the class runs in CPython, its methods compiled, as for a raise of it).
- A published compiled function, method or constructor (`--ext`, and a `--non-strict`
  native build's `__main__`) called with the wrong arguments raises CPython 3.13's exact
  `TypeError`: `missing 1 required positional argument: 'x'`, `takes from 2 to 3
  positional arguments but 4 were given`, and an unknown keyword's `Did you mean 'by'?`
  (also in compiled code's own keyword binding) -- issue #10.
- An f-string `=` field (`f"{x=}"`) in a `--non-strict` statement running in CPython
  printed the rewritten global read instead of `x=`.
### Known gaps
- v0.10 accept 3 (not met): pystache 0.6.8 and mistune 3.3.4 do not run their own
  benchmarks >= 1.2x faster than CPython (pystache 582k against CPython's 505k
  instructions per round, mistune 57.7M against 57.0M per pass). The levers left and their
  estimated savings are in [the v0.10 roadmap](https://github.com/rotnov/pycc-next/blob/main/docs/roadmap/v0.10.md); none reaches 1.2x
  alone.
- A subclass of a cross-module compiled subclass in a third module of a package runs as a
  Python subclass.
- Open rows of `docs/DEVIATIONS.md`.

### Speed
Bench run [41010653642](https://github.com/rotnov/pycc-next/actions/runs/41010653642) at
49f954c (ubuntu-24.04, Intel Xeon Platinum 8370C, medians of 11 interleaved
fresh-process runs; pydantic 15). Ratios are CPython's median over pycc-next's.

| workload | CPython | pycc-next | vs CPython | vs Cython 3.3.0 | vs mypyc |
|---|---|---|---|---|---|
| nbody | 2.606s | 0.077s | 33.9x | 63.5x faster | 29.5x faster |
| spectral_norm | 1.194s | 0.017s | 69.4x | 80.2x faster | 5.2x faster |
| diff-match-patch `diff_main` | 11.572s | 0.958s | 12.1x | 11.1x faster | does not compile |
| pypng read, five filters | 0.116s | 0.008s | 15.1x | 10.8x faster | does not compile |
| tomli `loads` x100 | 0.058s | 0.034s | 1.70x | 1.32x faster | 1.39x slower |
| pystache benchmark x1000 | 0.069s | 0.109s | 0.63x | 1.82x slower | does not compile |
| mistune `html` x20 | 0.149s | 0.179s | 0.83x | suite fails | does not compile |
| pydantic define | 8.54 ms | 10.49 ms | 0.81x | 1.19x slower | does not compile |
| pydantic schema | 19.03 ms | 23.03 ms | 0.83x | 1.42x slower | does not compile |

The run's job summary has every program, package workload and phase, with min-max.

Earlier versions: [0.9](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.9.md), [0.8](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.8.md),
[0.6-0.7](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.6-0.7.md), [0.4-0.5](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.4-0.5.md),
[0.1-0.3](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.1-0.3.md).
