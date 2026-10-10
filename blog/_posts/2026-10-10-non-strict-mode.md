---
title: "Untyped Python now compiles. The trick: don't pretend to know types you don't."
---

pycc-next has been **strict** from day one: no annotation, no build. That's great for code
you control and useless for the 90% of Python that was never annotated.

So today: **`--non-strict`**. 🐍

```python
def add(a, b):
    return a + b

print(add(1, 2), add("x", "y"), add(1.5, 2))
print(add(1, "a"))
```

```console
$ pycc build --non-strict add.py -o add && ./add
3 xy 3.5
TypeError: unsupported operand type(s) for +: 'int' and 'str'
```

Same output as CPython, **same exception, same message**.

## How it works 🔧

What I can't prove typed becomes a `PyObject*` and goes through the CPython C API:
`a + b` is `PyNumber_Add`, `d[k] = v` is `PyObject_SetItem`, `xs += more` is
`PyNumber_InPlaceAdd` (so a list really is extended in place). I don't reimplement
Python semantics - I call the thing that defines them.

Everything I *can* prove stays native. In `i = 0; while i < n: i += 1` with an untyped `n`,
`i` is still a machine integer. A name that genuinely changes type (`t = 0`, then
`t += x` with an object `x`) is widened to an object - like every Python name.

Typed and untyped code call each other: an object passed to `def f(n: int)` is checked at
the boundary (`TypeError: f() argument 1 must be int, not str`), then it's native inside.

## The honest part ⚠️

- Object code is **CPython-speed**, not Rust-speed. The win is that it *compiles* and
  mixes with fast typed code.
- A native build of non-strict code embeds CPython 3.14 (same machinery as foreign
  imports). `--ext` modules just work.
- The boundary check is a deviation: CPython would run `f` and fail (or not) later.
- Constructs I don't implement yet (`lambda`, generators) are still a clean `C0001`.

Strict stays the default. Non-strict is the on-ramp.
