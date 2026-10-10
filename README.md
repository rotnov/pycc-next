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
