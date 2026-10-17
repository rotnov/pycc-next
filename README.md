# pycc-next ⚡

**Your typed Python. Compiled. Checked. Fast.**

You already write type hints. pycc-next makes them *mean something*:

```python
def fib(n: int) -> int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)

print(fib("5"))
```

```
$ pycc check fib.py
error[T0021]: argument 1 of `fib` expects `int`, got `str`
 --> fib.py:6:11
  |
6 | print(fib("5"))
  |           ^^^ expected `int`, found `str`
```

Caught **before** it runs. Not in CI logs. Not in production. At compile time.

## Why pycc-next

|  | Python | mypy | Cython | **pycc-next** |
|---|---|---|---|---|
| Standard Python syntax | ✅ | ✅ | ⚠️ own dialect for speed | ✅ |
| Type errors stop the build | ❌ | ✅ checker only | ❌ | ✅ |
| Compiled to native code | ❌ | ❌ | ✅ | ✅ |
| Types drive the speed | ❌ | ❌ | ⚠️ `cdef` types | ✅ your annotations |

- 🛑 **Strict by default.** Type mismatch = compile error, like Rust. `Any` doesn't sneak through.
- 🐍 **Non-strict when you need it.** `--non-strict` compiles untyped code too: what can't be proven typed runs through CPython, with CPython's exact semantics.
- ⚡ **Native representations.** `int` becomes `i64`, `float` becomes `f64`, classes become structs. No boxing in hot loops.
- 🤝 **Plays with the ecosystem.** Target: a CPython extension module you `import`, calling numpy and torch as usual.
- 🧪 **CPython is the judge.** Correct means: same output, same exceptions as CPython 3.13.

## Status: 0.7, honestly 🧭

Works today: strict type checking with Rust-style diagnostics pointing at the offending
code; native compilation via LLVM; classes, exceptions, generators, closures, lists,
dicts, sets, comprehensions, `match`, arbitrary-precision `int`, typed numpy arrays;
`itertools`, `collections.deque`, `Counter` and `defaultdict` without CPython;
programs of several modules and packages; `@dataclass` (with `fields()`/`asdict()`/
`astuple()`/`replace()`), `enum.Enum`/`IntEnum`/`StrEnum`, sorting and hashing instances,
`functools.cache`/`lru_cache`; reference-counted memory;
CPython extension modules; `--non-strict` for untyped code, including classes inheriting
library classes, falling back to CPython one function or method at a time; a real
`int | float` type; `pip install` through the `pycc.build` PEP 517 backend. Linux
(x86_64, aarch64), macOS (arm64) and Windows (x86_64, MSVC). 9,500+ tests.

**Speed** (`python3 bench/run.py`, best of 7, shared 4-core Xeon 2.1 GHz under load from
other jobs, process start included; the Rust twins build with `rustc -O` for the default
target CPU and black-box their inputs, like the Python programs' inputs are opaque to
pycc-next):

| program | CPython 3.13 | pycc-next | Rust (`-O`) | vs CPython | vs Rust |
|---|---|---|---|---|---|
| fib(32) | 0.243s | 0.013s | 0.008s | **18.5x faster** | 1.7x slower |
| mandelbrot | 1.485s | 0.042s | 0.040s | **35.2x faster** | 1.0x slower |
| nbody | 2.299s | 0.068s | 0.027s | **33.6x faster** | 2.6x slower |
| sieve | 1.084s | 0.058s | 0.022s | **18.7x faster** | 2.7x slower |
| spectral_norm | 0.815s | 0.017s | 0.009s | **47.0x faster** | 1.9x slower |

**Real-world code** (`python3 bench/corpus.py`, 200 competitive-programming solutions,
compared with CPython on their tests): `--non-strict` compiles and passes all 200 with 0
wrong outputs (CI-gated), every top-level statement native (0 of 1191 in CPython
fallback); strict mode compiles and passes 190/200 unchanged — most of the rest use
dynamic values (mixed-type containers, an `int | None` used as an `int` without a `None`
test, a variable rebound to another type on some paths only) that strict mode refuses.

**Library code is not faster yet.** pydantic 2.13.5 built `--ext --non-strict`
(`bench/pydantic_check.py --time`, 809 of 1249 functions native, 5742 of its tests
passing) is *slower* than CPython running its sources: import 0.40x, model definition
0.70x, `model_json_schema()` 0.70x, validation round-trips 0.91x. Dynamic,
call-heavy code pays the CPython/compiled boundary and runs through the object protocol;
see [ROADMAP v0.7 accept 4](docs/ROADMAP.md) for the profile.

Known differences from CPython: [`docs/DEVIATIONS.md`](docs/DEVIATIONS.md).

## numpy arrays

Annotate a numpy array with its dtype and pycc-next compiles element access to machine
loads and stores -- no CPython call in the loop:

```python
import numpy as np
from numpy.typing import NDArray

def relax(src: NDArray[np.float64], dst: NDArray[np.float64]) -> None:
    n, m = src.shape
    for i in range(1, n - 1):
        for j in range(1, m - 1):
            dst[i, j] = 0.25 * (src[i - 1, j] + src[i + 1, j] + src[i, j - 1] + src[i, j + 1])
```

- **Types:** `NDArray[np.float64]`, `NDArray[np.int64]`, `NDArray[np.bool_]` (also
  spelled `npt.NDArray[...]`, or numpy's `np.ndarray[tuple[int, int], np.dtype[np.float64]]`),
  of 1 or 2 dimensions. The dimension count is read off how the array is indexed
  (`a[i, j]`, `a[i][j]`, `n, m = a.shape`); with no such use it is 1. Arrays from
  `np.zeros`/`ones`/`empty`/`full`/`array`/`arange`/`zeros_like`/`.copy()` are typed
  without an annotation when the dtype and shape are evident from the call.
- **Native:** `a[i]`, `a[i, j]`, `a[i][j]` reads and writes (negative indices,
  numpy's own `IndexError` text, read-only arrays refused with numpy's `ValueError`),
  `.shape`, `.size`, `.ndim`, `len(a)`, `for x in a` over a 1-D array. Any memory
  layout works (views, transposes, non-contiguous strides).
- **Checked at the boundary:** a value entering a typed parameter or binding must be a
  `numpy.ndarray` of that dtype and dimension count (`TypeError` otherwise). A local's
  shape and data pointer then stay in registers until a call that may run CPython code
  (which could `resize` the array), after which the array is checked again; a global or
  attribute is checked at each use (inline, a few instructions). Everything else on an
  array (`a.sum()`, `np.dot(a, b)`, slices) is an ordinary CPython call.
- Works in `--ext` modules (the caller's array is shared, so writes are visible to it)
  and in native builds (numpy is bundled from `pycc.lock`).
- Elements read as Python `float`/`int`/`bool`, not numpy scalars: see
  [`docs/DEVIATIONS.md`](docs/DEVIATIONS.md).

**Speed** (`python3 bench/run.py stencil2d prefix_sum_np`, kernel time without numpy's
import, best of 7, Xeon 2.8 GHz shared with other jobs):

| program | CPython 3.13 + numpy | pycc-next | Rust (`-O`) | vs CPython | vs Rust |
|---|---|---|---|---|---|
| stencil2d (200x300 Jacobi, 120 sweeps) | 4.913s | 0.011s | 0.004s | **441.6x faster** | 2.5x slower |
| prefix_sum_np (400k `int64`, 25 rounds) | 4.048s | 0.031s | 0.017s | **129.9x faster** | 1.8x slower |

An `int64` element joins `x += a[i]` and comparisons as a machine integer (an `int` is
made of it only when the result leaves the inline range), so prefix_sum_np's running sum
costs two overflow checks per add.

## Programs of several modules

A program can span several `.py` files and packages: `import m`, `import pkg.sub`,
`import pkg.sub as s`, `from pkg.sub import f`, `from m import f as g`, relative
imports (`from . import x`, `from ..util import f`) and `__init__.py`. pycc-next finds
them next to the entry file (a package's root is found by walking up while directories
hold an `__init__.py`; a `pycc.toml` names it explicitly) and compiles the whole program
as one unit:

- each module keeps its own namespace: two modules may both define `main` or `CONFIG`,
  and each module's `__name__` is its own dotted name;
- `m.f(...)`, `m.CONST`, `m.Class(...)` and `from m import f` reach the definition
  directly: a call between modules is a direct native call;
- each module body runs once, at the first `import` that reaches it, in CPython's import
  order (`pkg/__init__.py` before `pkg/sub.py`);
- `from m import x` of a variable copies its value, as in CPython, while `m.x` reads
  the current one.

```console
$ pycc build app/main.py -o app/main && ./app/main     # strict, native
$ pycc build app/main.py -o app/main --non-strict      # untyped code too
$ pycc build app/main.py -o main --ext                 # one CPython extension module
```

With `--ext` the artifact is the entry module: `import main` from CPython runs the whole
program, with every project module it imports compiled in. Those modules are private to
the extension (not attributes of it, and not in `sys.modules`); only the entry module's
own definitions are exported. To ship each module of a package as its own extension,
build them one by one with `pycc build pkg/mod.py --ext --foreign-relative-imports`:
their relative imports then bind the other modules through CPython. Imports of anything
else (the standard library, third-party packages) work as in a single file. Modules may import
each other (a circular import sees the partially initialized module, as in CPython). A
module used as a value, the entry module imported back by its own imports and a project
import inside an `if`/`try` block are compile errors; the details are in [`docs/DEVIATIONS.md`](docs/DEVIATIONS.md).

### Real packages, unchanged

`python3 bench/package_check.py` (CI-gated) downloads four pinned, permissively licensed
pure-Python packages, builds each of their modules unchanged as its own extension,
removes the `.py` files and runs the package's own unittest suite with CPython against
the compiled modules (`python3 bench/package_check.py tomli --time` checks one and times
its hot path; `--instructions` counts the instructions one call of the hot path executes
under callgrind and fails over a documented bound, which CI also gates):

- [diff-match-patch](https://pypi.org/project/diff-match-patch/) 20241021 (Apache-2.0,
  Google's diff/match/patch algorithms, pinned by sdist sha256);
- [tomli](https://pypi.org/project/tomli/) 2.5.0 (MIT, the TOML parser `tomllib` comes
  from; its sdist has no tests, so the GitHub tag archive is used, pinned by its sha256
  and by that of the files used);
- [pypng](https://pypi.org/project/pypng/) 0.20220715.0 (MIT, a PNG encoder and decoder
  in one module, `png`, pinned by sdist sha256);
- [mistune](https://pypi.org/project/mistune/) 3.3.4 (BSD-3-Clause, a CommonMark Markdown
  parser and renderer in 38 modules, pinned by sdist sha256).

An archive member that is not a regular file or a directory (a link, a device) is
refused before extraction.

For diff-match-patch:

```console
$ pycc build diff_match_patch/__init__.py -o diff_match_patch/__init__ \
      --ext --non-strict --foreign-relative-imports            # likewise for every module
$ python3 -m unittest diff_match_patch.tests                    # 28 tests, OK
```

A package `__init__.py` builds to `pkg/__init__.abi3.so`, imported as the package. Every
function and method of the package compiles (`--report-fallbacks` reports none; the
`__init__`'s re-exporting imports also run as CPython imports). Its untyped text
functions run natively on `str` (guarded native specializations, see Modes): the
package's own speed test (`diff_main` on two ~30 KB texts) takes 1.03 s compiled
against 12.3 s in CPython (**12x faster**, best of 5; 11x-13x over repeated runs;
v0.3 was 1.7x).

tomli is typed code written for mypyc (`from __future__ import annotations`, aliases
imported only under `if TYPE_CHECKING:`, `Final` constants, frozenset tables,
keyword-only parameters, `tuple[int, str]` results). Its four modules build unchanged
and its own 18 tests pass (one is skipped by tomli itself). 37 of its 40
functions compile; 3 run in CPython (`--report-fallbacks`: the exception class's
`__init__`, which imports, and `make_safe_parse_float()` and its closure). A function whose
signature the boundary cannot carry (a `tuple[int, str]` result, a `frozenset`
parameter) is compiled for the module's own callers and runs in CPython only when Python
calls it (`twin:` in the report). `loads()` of tomli's benchmark document runs
**1.4x-1.7x faster** than in CPython: `python3 bench/package_check.py tomli --time --runs 15`
times 100 calls, best of 15 runs for each side on the same machine; v0.4 measured
0.036 s compiled against 0.059 s, and the current tree 0.052 s against 0.074 s on a
busier box (2.54M against 4.56M instructions per call under callgrind, the stable
measure; v0.4: 2.60M; it was 0.40x before the work above).
What it takes: the parsing tables are native frozensets and the text is scanned
natively (`while src[pos] in chars: pos += 1` is one scan); a function returning
`pos, value` hands both back without a tuple, its callers unpacking them directly
(multi-value returns); a `str | None` local stays a native `str`; an object compared
with a `str` literal, or tested against a frozenset of `str`, is compared by its
bytes; its keys (`tuple[str, ...]`) are native tuples, recycled through the runtime's
free lists as CPython recycles its tuples, and `tuple[int, Any]` results keep the
position native. Its documents and flag tables (`dict[str, Any]`, mutable and
returned to the caller, so they stay CPython objects) and regular-expression matches
are still reached through the C API, and each of its modules calls the others through
CPython: under callgrind, CPython itself (the regular expression engine, the dict
operations, the `int`/`datetime` conversions) is 1.14M of the 2.54M instructions per
call, the reason the original v0.5 goal of 3x (1.5M) was withdrawn.

pypng is untyped, byte-crunching code: its `png` module builds unchanged and its own 103
tests pass against it (one skips itself: it runs the sdist's shell scripts). Decoding
undoes a filter on every scanline in pure-Python loops over `bytearray`s
(`undo_filter_paeth(filter_unit, scanline, previous, result)`); those loops get guarded
native specializations taking the `bytearray`s as they are, reading and writing their
buffers in place (`--report-fallbacks`: `specialized: undo_filter_paeth ... scanline:
bytearray, previous: bytearray, result: bytearray`). Reading a 256x256 RGB image whose rows
use all five filter types runs **10x faster** than in CPython: `python3
bench/package_check.py pypng --time --runs 5` measured 0.015 s against 0.157 s for five
`Reader.read()`s. 78 of its 85 functions compile (64 before v0.7's method-level fallback, 15 in v0.5;
`--report-fallbacks`),
`Reader` among them: its chunk handlers, which it looks up by name
(`getattr(self, "_process_" + kind)`), its generator methods feeding each other, the
nested `def convert()` each branch defines, and `bytes` literals, `*args` calls and
`except struct.error:`. Its sub-byte rows are unpacked by a specialization taking
`self.bitdepth` as a native `int` into a native `bytearray`. Still in CPython: seven
functions (`Writer.__init__` and `from_array`, which `del` a CPython object, two using
generator expressions, a local `import`, a read of `__file__`, and a membership test of
an object in a list of tuples); the `Writer` and exception classes are created by
CPython (a class used as a default value, a custom `__str__`, attribute bases), their
other methods compiled. Decoding every PngSuite image its tests use (`--time`'s second
measurement) runs **3.2x faster** than CPython (19 ms against 60 ms for five passes,
best of 15 interleaved runs; 4.6x fewer instructions under callgrind). The filter loops'
`int` locals are proven to stay small (byte items, `& 0xFF`, `abs()`, `range()` bounds),
so they compute on tagged words without overflow checks; what remains is mostly CPython
itself (`zlib`, `struct.unpack` and `array` for the 16-bit images, the palette's tuples)
and per-row object calls, not the filters.

mistune is typed, class-based library code: a block parser, an inline parser, plugins,
directives and renderers, in 38 modules that call each other through their classes,
with tokens as dicts and dispatch by name (`getattr(self, name)`, a dict of lambdas).
All 38 build unchanged (in parallel, as `pycc.build` builds them) and its own 1155 tests
pass against them; 354 of its 413 functions compile (`--report-fallbacks`). It is
**slower** than CPython: rendering every document of its own benchmark
(`benchmark/cases`) with `mistune.html` takes 0.22 s against 0.15 s for 20 passes
(0.66x), and its "full" parser with every plugin 0.61x; under callgrind 77.7M
instructions per pass against CPython's 56.8M. Its classes are created by CPython
(`Generic[ST]` bases, bases from another module, lambdas), so every method is a
compiled body behind a Python forwarder, every `self.x` is a generic
`PyObject_GetAttr` where CPython's interpreter uses a specialized instruction, and
its values (tokens, states, matches) are CPython objects reached through the C API:
compiled code runs a third of the bytecode CPython runs and saves 13M instructions per
pass there, and pays 34M back in attribute lookups, method objects, stable-ABI reference
counting calls and conversions. What it took to compile at all is general: `typing`'s
`Optional`/`Union` and its unmodeled names (`Match`, `MutableMapping`), string
annotations of `TYPE_CHECKING`-only imports, private module functions exported, `str`
crossing without copies.

## Use pycc in your project

`pycc.build` is a PEP 517 build backend: with it, `pip install .` and `pip wheel .`
compile your project's modules to abi3 CPython extension modules (one `--ext` module
per `.py` file, `--foreign-relative-imports`, `--release`), keep the modules you name in
Python, and write a wheel tagged `cp313-abi3-<platform>` (CPython 3.13 and newer):
`manylinux_<glibc>_<arch>` on Linux (the build machine's glibc; `linux_<arch>` if a
module links a library manylinux does not allow), `macosx_<N>_0_arm64` on macOS (the
minimum macOS the modules record) and `win_amd64` on Windows. Select it in `pyproject.toml`:

```toml
[build-system]
requires = ["pycc-build"]
build-backend = "pycc.build"

[project]                          # static PEP 621 metadata (no `dynamic`)
name = "geo"
version = "1.0"
requires-python = ">=3.13"

[tool.pycc]
strict = true                      # default; false builds every module --non-strict
pure-python = ["geo.cli"]          # modules (a package name covers its subpackages) kept as .py
fallback-to-python = false         # true, or a list of modules: ship as .py if pycc refuses them
extra-args = []                    # more `pycc build` flags for every module
# packages = ["geo"], py-modules = ["m"], source-dir = "src": what ships, when the
# default discovery does not find it (a src/ layout, else the package or module named
# after the project, else the flat layout's packages and modules but tests, docs, ...).

[tool.pycc.overrides."geo.__init__"]   # per module, or per package and its subpackages
strict = false
```

The backend ships in the release archive as `python/pycc_build-X.Y.Z-py3-none-any.whl`
(no PyPI download): either let pip's isolated build environment install it from there,
or install it into your environment once and build without isolation. It finds the
compiler as `$PYCC` (the `pycc` executable or the unpacked archive's directory), else
`pycc` on `PATH`; it runs it with `PYCC_PYTHON` set to the interpreter pip runs, unless
you set it.

```console
$ export PYCC=$HOME/pycc-vX.Y.Z-x86_64-linux
$ pip wheel . --no-index --find-links $PYCC/python -w dist   # isolated build, offline
$ pip install $PYCC/python/pycc_build-*.whl                    # or: install the backend,
$ pip install . --no-build-isolation                           # then build in place
```

A module pycc cannot compile fails the build with pycc's diagnostics (and says which
setting keeps it in Python); `pip -v` also shows each module's build time and, for a
`--non-strict` module, what runs in CPython (`--report-fallbacks`). The build runs no
project code: metadata is read from `pyproject.toml`, sources are only copied and
compiled, symbolic links in the sources are refused, and everything is written under
the directory pip hands the backend. `pip install -e .` (PEP 660) installs the sources
uncompiled, for development; `python -m build --sdist` (or pip) makes an sdist of the
sources, `pyproject.toml`, readme and license files (`sdist-include` adds more globs).

In strict mode a compiled module publishes its functions, classes and evidently native
constants, but not what it imports (`docs/DEVIATIONS.md`): a package `__init__.py` that
re-exports names (`from .core import f`) should build `--non-strict`, as in the
override above, or stay Python.

CI builds the three packages above this way (`python3 bench/wheel_check.py`): `pip
wheel` with the archive's backend and compiler, a fresh venv, a check that each module
is an extension module, and each package's own test suite against the installed wheel.
Their `pyproject.toml` is replaced by one selecting `pycc.build` (they use flit_core
and setuptools): that is the test harness, not a change to the packages.

## Modes

- **Strict** (default): `Any` is refused (`T0002`) and a type mismatch is a compile
  error. In a native build an unannotated function or method (`__init__` aside) gets
  one signature inferred from its call sites, as a private helper does: every call
  site must agree and the body must pin the return, otherwise it is `T0001` and needs
  annotations. An `--ext` module's public functions must be annotated (`T0001`),
  since Python may call them with anything.
  A value that is an `int` at one point and a `float` at another is an `int | float`
  -- annotated, or inferred for a name, an accumulator (`s = 0` ... `s += 0.5`), a
  `float('inf')` sentinel an `int` replaces, a list display or `.append` of both, a
  dict display's values: one tagged machine word (an `int`, or a `float` held inline
  for magnitudes between about `1e-77` and `1e77`), with CPython's semantics for
  whichever it holds. Pure `int` and pure `float` code is unchanged.
- **Non-strict**: `pycc build|run|check --non-strict`, or `[tool.pycc] strict = false`
  in a `pyproject.toml` next to the entry file (`[build] strict = false` in a
  `pycc.toml` works too). A missing annotation and `Any` become a CPython object
  (`PyObject*`); operations on it (arithmetic, comparisons, calls, attribute and
  item access and stores, iteration) go through the CPython C API, so results and
  exceptions are CPython's own. Provable types stay native: local inference is
  unchanged, and a name rebound with a different type becomes an object. In a
  native build an unannotated function's signature is inferred from its call sites
  (a parameter or return nothing pins is an object); an `--ext` module's stays an
  object, since Python may call it with anything -- but one that loops gets a *guarded
  native specialization* when its body implies `str`, `int` or `bytearray` for
  parameters (indexed, sliced, `len()`, `str` methods; indices, slice bounds, `range()`
  arguments; items used as numbers, `s[i] ^ 0xff`): called with exactly those types it
  runs natively (a `bytearray` in place, not copied), otherwise on the objects; one the
  checker refuses is simply not specialized (`--report-fallbacks` lists both). A name one
  function rebinds with another type becomes an object in that function only. A native
  build of such a program embeds CPython (as a foreign import does); `--ext` just
  works, and publishes the module's variables and imports as module attributes and
  its classes' constants as class attributes. Known gaps are listed in
  `docs/DEVIATIONS.md`.
- **Per-function fallback** (non-strict only): a construct pycc cannot compile yet (a
  lambda used as a value, ...) sends only the smallest enclosing function or method to
  CPython, kept as Python source and run by CPython's own `exec` in the module's
  namespace, at its place in the module. A class whose statement CPython has to run
  (it inherits a CPython class such as `threading.Thread`, `collections.Counter`,
  `argparse.Action` or a library exception, has a metaclass, descriptors, class
  attributes of any type, decorated methods or properties) is created by CPython, and
  each of its methods' bodies is still compiled: the `def` in the class body forwards
  to a compiled function (`self` crosses as the CPython instance, `super()` and
  `__class__` keep working), so Python code subclassing the class, metaclass hooks and
  `__init_subclass__` behave as in CPython. In an `--ext` module a compiled class whose
  method cannot compile stays a native class, that method attached to it as a CPython
  function. A decorated top-level function keeps its `def` (decorators, defaults,
  signature) in CPython and its body compiled. `--report-fallbacks` reports each
  (`fallback: method ...`, `lifted: function ...`, `fallback: class ... its methods
  compiled`). Library-typed annotations (`m: re.Match`), general unions, `X | None` of
  objects, the `with` statement and starred expressions compile natively on objects. Compiled code calls a fallback
  function as a CPython object; fallback code calls compiled functions as the
  module's published callables (an unannotated function that fallback code calls
  is compiled with object parameters instead of inferred ones; one with a parameter
  default runs in CPython, as published callables do not apply defaults yet). An
  unannotated definition nothing in the entry module mentions (a template's unused
  `def list2d(a, b, c)`, a method no attribute access names) is not compiled at
  all: it can never run, so it neither falls back nor costs anything. A
  module global that fallback code binds lives in the module dict, so both sides
  see one value; one it only reads stays native, each compiled store copied into
  the dict (a list or other container still lives in the dict, built there as a
  CPython list when it is a fresh display or `[x] * n`); an import, a lambda, or an alias
  such as `input = sys.stdin.readline`, that fallback code reads stays compiled
  and also runs in CPython. A builtin call pycc does not compile for its arguments
  (`min` of an object, `set()`, `pow(a, b, m)`, ...) is a call of CPython's own
  builtin from compiled code rather than a fallback, as is a builtin used as a value
  (`defaultdict(int)`), `exit()`, and a `str`/`int`/`float` method pycc does not
  compile for its arguments (`"\n".join(objects)`). Exceptions cross the
  boundary both ways with CPython's type and message, and tracebacks keep their line
  numbers. `--report-fallbacks` prints what fell back and why; strict mode keeps
  refusing with the same diagnostics.

### Which CPython

Everything that needs CPython -- an `--ext` module's headers, a native build that
embeds an interpreter (a foreign import, or non-strict code), and the oracle in the
tests -- uses the interpreter named by `PYCC_PYTHON`, default `python3`
(`python.exe` on Windows): any CPython 3.13 or newer with a shared libpython (or a
static one with `--static-libpython`). The extension shim targets the stable ABI
(`Py_LIMITED_API` 3.13). An embedded build copies that interpreter's standard
library into `OUT.pycc/`.

The shim is compiled in two parts: a small one per module, and its fixed half, which is
the same for every module and is compiled once and kept in a build cache, keyed by its
text, the compile flags, the CPython headers and the C compiler -- `PYCC_CACHE_DIR`, else
`$XDG_CACHE_HOME/pycc` or `~/.cache/pycc` (`~/Library/Caches/pycc` on macOS,
`%LOCALAPPDATA%\pycc` on Windows); an empty `PYCC_CACHE_DIR` turns the cache off.

📓 Follow the build: **https://rotnov.github.io/pycc-next/** — written by the AI agent
building it, failures included.

## Try it

### From a release (x86_64 and aarch64 Linux, arm64 macOS, x86_64 Windows)

Download the archive for your machine from
[Releases](https://github.com/rotnov/pycc-next/releases) --
`pycc-vX.Y.Z-x86_64-linux.tar.gz`, `pycc-vX.Y.Z-aarch64-linux.tar.gz` (arm64
Linux), `pycc-vX.Y.Z-aarch64-macos.tar.gz` (Apple silicon) or
`pycc-vX.Y.Z-x86_64-windows.zip` -- and unpack it anywhere:

```console
$ tar xzf pycc-vX.Y.Z-x86_64-linux.tar.gz
$ pycc-vX.Y.Z-x86_64-linux/bin/pycc build fib.py -o fib --release && ./fib
```

The archive holds `bin/pycc`, its runtime library `lib/pycc/libpycc_rt.a`, the
build backend's wheel `python/pycc_build-X.Y.Z-py3-none-any.whl` (see "Use pycc in
your project"), and this README, `LICENSE` and `CHANGELOG.md`. Keep `bin/` and `lib/` together; the
directory itself can live anywhere (or symlink `bin/pycc` into your `PATH`). LLVM
is linked in. What you need on the machine:

- **Linux x86_64 or aarch64 with glibc 2.39+** (Ubuntu 24.04, Debian 13, Fedora 40 or newer),
  plus `libstdc++`, `zlib`, `zstd` and `libffi` 8 (present on those systems);
  or **macOS 26+ (Apple Silicon)** -- the archive is built for macOS 26.0 and
  needs no Homebrew libraries. Programs pycc builds target the macOS it runs on
  (`cc`'s default), or `MACOSX_DEPLOYMENT_TARGET` when set (pycc then emits
  code for that version too). Intel Macs are not supported.
- **A C toolchain** with `cc` on `PATH` -- pycc links every program with it. On
  Linux gcc or clang (e.g. `apt install build-essential`); on macOS the Xcode
  Command Line Tools (`xcode-select --install`). Without it `pycc build` stops
  with "could not run the linker driver" and says what to install.
- **CPython 3.13+ with its headers and shared libpython** (e.g. `apt install
  python3-dev`; on macOS the python.org installer, Homebrew `python@3.13`, `uv`
  or `pyenv`) for `--ext` modules and for programs that need CPython at run time
  (`--non-strict` code, standard-library imports). Named by `PYCC_PYTHON`,
  default `python3`. A plain typed program needs no Python at all.

**Windows (x86_64):** the zip holds `bin\pycc.exe` and `lib\pycc\pycc_rt.lib`. It
needs Windows 10 or newer and, at build time:

- **the MSVC toolchain** -- Visual Studio 2022 or newer, or its Build Tools, with
  "Desktop development with C++" (MSVC's `link.exe`, the Windows SDK and the C
  runtime libraries); run pycc from a "Developer PowerShell" / "Developer Command
  Prompt", which sets `LIB`, `INCLUDE` and `VCToolsInstallDir`;
- **clang** -- pycc compiles its C sources and drives `link.exe` with it: `PYCC_CLANG`,
  else `clang.exe` on `PATH`, else Visual Studio's "C++ Clang Compiler for Windows"
  component, else `%ProgramFiles%\LLVM\bin\clang.exe` (the LLVM installer);
- **CPython 3.13+ from python.org** (its installer ships `libs\python3.lib`, which
  `--ext` modules link) for `--ext` modules and programs that need CPython at run
  time; `PYCC_PYTHON`, default `python.exe`. Extension modules are abi3 `.pyd` files
  (`pycc build m.py -o m --ext` writes `m.pyd`), loaded by CPython 3.13 and newer. A
  program that embeds CPython is an `OUT.exe` stub with its interpreter in `OUT.pycc\`
  (`docs/DEVIATIONS.md` lists the Windows gaps).

On macOS a browser download is quarantined by Gatekeeper; clear the flag once
after unpacking (`xattr -dr com.apple.quarantine pycc-vX.Y.Z-aarch64-macos`) or
download with `curl -LO`, which sets no quarantine flag.

### From source

Rust 1.97.1 and LLVM 22 (Linux: apt.llvm.org's `llvm-22-dev`; macOS:
`brew install llvm@22`):

```console
$ export LLVM_SYS_221_PREFIX=/usr/lib/llvm-22   # macOS: $(brew --prefix llvm@22)
$ cargo build --release
$ ./target/release/pycc build hello.py -o hello && ./hello
$ scripts/package.sh dist/pycc   # the release layout, from target/release
```

On macOS `package.sh` refuses a `pycc` that links Homebrew's `libzstd.dylib`;
the release build links `libzstd.a` instead (`scripts/macos_llvm.sh`).

On Windows, from Git Bash in a developer environment: LLVM's own release
`clang+llvm-22.1.8-x86_64-pc-windows-msvc.tar.xz` (it carries `llvm-config.exe` and the
static libraries; `scripts/windows_llvm.sh` downloads it pinned by sha256, as CI does),
`LLVM_SYS_221_PREFIX` naming it, and `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER`
naming MSVC's `link.exe` (Git Bash's own `link` shadows it on `PATH`); then
`cargo build --release` and `scripts/package.sh` as above.

### Testing against CPython

CPython 3.13 is the oracle everywhere: `cargo test --release --workspace --
--include-ignored` builds every fixture natively and as an extension module and compares
it with `python3`; `python3 bench/corpus.py --flags=--non-strict --require-correct` does
the same for 200 real-world programs.

The differential fuzzer (`fuzz/`) looks for the differences nobody wrote a test for. It
generates typed programs (boundary values, big ints, `nan`/`-0.0`, non-ASCII strings,
format specs, containers, classes, dataclasses, generators with `send`/`throw`,
exceptions, `match`, `*args`/`**kwargs`, `collections`, `itertools`, ...), runs each
under CPython and as native strict, `--non-strict`, `--ext` and `--ext --non-strict`
builds, and compares stdout, the uncaught exception and the exit status. A refusal is
not a mismatch, but every refusal is counted per feature and mode; a compiler crash is
a finding. Each finding is reduced (delta debugging over the AST) to a minimal program:

```console
$ python3 fuzz/fuzz.py --seconds 600 -j 4 --seed 7      # findings in fuzz/findings/
$ python3 fuzz/fuzz.py --replay 7:123                   # one case, verbosely
$ python3 fuzz/gen.py 7 123                             # print that case's program
```

CI runs it for 2 minutes on every push and for 30 minutes nightly
(`.github/workflows/fuzz.yml`), failing on any finding. A difference
`docs/DEVIATIONS.md` documents is recognized and counted, not reported; every program
behind a finding fixed so far is a regression test (`tests/fuzz_regressions.rs`).

## License

MIT
