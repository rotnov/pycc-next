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

## Status: 0.2, honestly 🧭

Works today: strict type checking with Rust-style diagnostics pointing at the offending
code; native compilation via LLVM; classes, exceptions, generators, closures, lists,
dicts, sets, comprehensions, `match`, arbitrary-precision `int`, typed numpy arrays;
programs of several modules and packages; `@dataclass`, `enum.Enum`/`IntEnum`/`StrEnum`,
`functools.cache`/`lru_cache`; reference-counted memory;
CPython extension modules; `--non-strict` for untyped code. 9,500+ tests.

**Speed** (`python3 bench/run.py --runs 11`, best of 11, shared 4-core Xeon 2.1 GHz, process
start included; the Rust twins build with `rustc -O` for the default target CPU and black-box
their inputs, like the Python programs' inputs are opaque to pycc-next):

| program | CPython 3.13 | pycc-next | Rust (`-O`) | vs CPython | vs Rust |
|---|---|---|---|---|---|
| fib(32) | 0.354s | 0.015s | 0.008s | **24.0x faster** | 1.8x slower |
| mandelbrot | 1.810s | 0.061s | 0.055s | **29.5x faster** | 1.1x slower |
| nbody | 2.584s | 0.084s | 0.034s | **30.9x faster** | 2.4x slower |
| sieve | 1.776s | 0.057s | 0.028s | **31.3x faster** | 2.1x slower |
| spectral_norm | 1.175s | 0.021s | 0.012s | **56.4x faster** | 1.8x slower |

**Real-world code** (`python3 bench/corpus.py`, 200 competitive-programming solutions,
compared with CPython on their tests): `--non-strict` compiles and passes all 200 with 0
wrong outputs (CI-gated), 95% of top-level statements native and the rest per-statement
CPython fallback; strict mode compiles 124/200 unchanged — most of the rest use dynamic
values (mixed-type containers, CPython-only builtins) that strict mode refuses.

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
else (the standard library, third-party packages) work as in a single file. A module used
as a value, an import cycle and a project import inside an `if`/`try` block are compile
errors; the details are in [`docs/DEVIATIONS.md`](docs/DEVIATIONS.md).

## Modes

- **Strict** (default): `Any` is refused (`T0002`) and a type mismatch is a compile
  error. In a native build an unannotated function or method (`__init__` aside) gets
  one signature inferred from its call sites, as a private helper does: every call
  site must agree and the body must pin the return, otherwise it is `T0001` and needs
  annotations. An `--ext` module's public functions must be annotated (`T0001`),
  since Python may call them with anything.
- **Non-strict**: `pycc build|run|check --non-strict`, or `[tool.pycc] strict = false`
  in a `pyproject.toml` next to the entry file (`[build] strict = false` in a
  `pycc.toml` works too). A missing annotation and `Any` become a CPython object
  (`PyObject*`); operations on it (arithmetic, comparisons, calls, attribute and
  item access and stores, iteration) go through the CPython C API, so results and
  exceptions are CPython's own. Provable types stay native: local inference is
  unchanged, and a name rebound with a different type becomes an object. In a
  native build an unannotated function's signature is inferred from its call sites
  (a parameter or return nothing pins is an object); an `--ext` module's stays an
  object, since Python may call it with anything. A native
  build of such a program embeds CPython (as a foreign import does); `--ext` just
  works. Known gaps are listed in `docs/DEVIATIONS.md`.
- **Per-function fallback** (non-strict only): a top-level function, class or
  statement that pycc cannot compile yet (a lambda used as a value, ...) is
  kept as Python source and run by CPython's own `exec` in the
  module's namespace, at its place in the module. Compiled code calls a fallback
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

📓 Follow the build: **https://rotnov.github.io/pycc-next/** — written by the AI agent
building it, failures included.

## Try it

### From a release (x86_64 and aarch64 Linux, arm64 macOS)

Download the archive for your machine from
[Releases](https://github.com/rotnov/pycc-next/releases) --
`pycc-vX.Y.Z-x86_64-linux.tar.gz`, `pycc-vX.Y.Z-aarch64-linux.tar.gz` (arm64
Linux) or `pycc-vX.Y.Z-aarch64-macos.tar.gz` (Apple silicon) -- and unpack it
anywhere:

```console
$ tar xzf pycc-vX.Y.Z-x86_64-linux.tar.gz
$ pycc-vX.Y.Z-x86_64-linux/bin/pycc build fib.py -o fib --release && ./fib
```

The archive holds `bin/pycc`, its runtime library `lib/pycc/libpycc_rt.a`, and
this README, `LICENSE` and `CHANGELOG.md`. Keep `bin/` and `lib/` together; the
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

## License

MIT
