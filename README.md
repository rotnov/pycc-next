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
 --> fib.py:1:1
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

## Status: pre-alpha, honestly 🧭

Works today: strict type checking with Rust-style diagnostics, native compilation via
LLVM, classes, exceptions, containers, comprehensions, `match` — 8,800+ tests.
`fib(32)` runs ~3x faster than CPython including process start.

Coming next: numpy arrays as native types, published benchmarks against CPython
**and** hand-written Rust.

## Modes

- **Strict** (default): every public function is annotated (`T0001`), `Any` is
  refused (`T0002`), and a type mismatch is a compile error.
- **Non-strict**: `pycc build|run|check --non-strict`, or `[tool.pycc] strict = false`
  in a `pyproject.toml` next to the entry file (`[build] strict = false` in a
  `pycc.toml` works too). A missing annotation and `Any` become a CPython object
  (`PyObject*`); operations on it (arithmetic, comparisons, calls, attribute and
  item access and stores, iteration) go through the CPython C API, so results and
  exceptions are CPython's own. Provable types stay native: local inference is
  unchanged, and a name rebound with a different type becomes an object. A native
  build of such a program embeds CPython (as a foreign import does); `--ext` just
  works. Known gaps are listed in `docs/DEVIATIONS.md`.
- **Per-function fallback** (non-strict only): a top-level function, class or
  statement that pycc cannot compile yet (a lambda, a generator, a nested function,
  `global`, ...) is kept as Python source and run by CPython's own `exec` in the
  module's namespace, at its place in the module. Compiled code calls a fallback
  function as a CPython object; fallback code calls compiled functions as the
  module's published callables. A module global that fallback code reads or binds
  lives in the module dict, so both sides see one value. Exceptions cross the
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

Rust 1.97.1 and LLVM 22:

```console
$ export LLVM_SYS_221_PREFIX=/usr/lib/llvm-22
$ cargo build --release
$ ./target/release/pycc build hello.py -o hello && ./hello
```

## License

MIT
