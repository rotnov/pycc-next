---
title: "I deleted list[int]. Now every list compiles."
---

Yesterday `sieve`, `spectral_norm` and `nbody` didn't compile. The reason was embarrassing:
pycc only knew **one** list type, `list[int]`, hard-wired from the type checker down to
the runtime (`PyIntListObj`, `pycc_rt_int_list_*`). `[True] * n`? Error. `list[float]`?
Error. A list of class instances? Error.

## The rewrite 🔨

The fix wasn't "add `list[float]`". It was **one** list:

- 📦 every element is one 8-byte word in its **native** form: an `int` word, `f64` bits,
  `0/1`, or a pointer (`str`, class instance, nested list)
- 🏷️ the runtime object records the element kind, so equality, `repr` and
  refcounting of `str` elements are generic
- ⚡ `len(xs)`, `xs[i]`, `xs[i] = v` and `for x in xs` are **inlined** loads from the
  object's `{len, data}` header - no runtime call, `IndexError` on a cold branch

Negative indices, full CPython slicing, `+`, `*`, `in`, `insert`, `pop(i)`,
`print(xs)` - all byte-for-byte against CPython 3.13, plus `round()` so the
benchmarks can print their results.

## Numbers 📊

Fresh run, 4-core Xeon @ 2.1 GHz shared with other jobs, best of 3:

| program | CPython | pycc-next | Rust | vs CPython | vs Rust |
|---|---|---|---|---|---|
| sieve | 1.528s | 0.452s | 0.036s | **3.4x faster** | 12.5x slower 😬 |
| spectral_norm | 0.919s | 0.234s | 0.009s | **3.9x faster** | 25x slower |
| nbody | 2.594s | 0.801s | 0.023s | **3.2x faster** | 35x slower |

They compile and match CPython. They are **not** fast yet. The list accesses are inline
now; what's left in the hot loops is the same villain as last time - every `i * i`,
`j += i` and `i < n` is a call into the runtime, and every `b.x` attribute read is too.

## Still refused (on purpose) 🚫

- `xs += ys` - CPython mutates in place; I'd rather refuse than rebuild the list
- printing a list of class instances - I can't call `__repr__` from the runtime yet

## Next 🛠️

Inline tagged-int arithmetic and inline attribute slots. Then this table again.
