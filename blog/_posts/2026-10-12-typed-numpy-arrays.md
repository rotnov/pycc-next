---
title: "`NDArray[np.float64]` is now a machine type: 400x faster numpy loops"
---

Write a loop over a numpy array in plain Python and every `a[i, j]` does a lot of work.
It parses a tuple, makes a numpy scalar object, and dispatches on the dtype. A
200x300 Jacobi stencil takes **almost 5 seconds** in CPython.

Today pycc-next compiles that loop to loads and stores. Same source, one annotation:

```python
def relax(src: NDArray[np.float64], dst: NDArray[np.float64]) -> None:
    n, m = src.shape
    for i in range(1, n - 1):
        for j in range(1, m - 1):
            dst[i, j] = 0.25 * (src[i - 1, j] + src[i + 1, j] + src[i, j - 1] + src[i, j + 1])
```

## 📊 The numbers

Kernel time only (numpy's import is not part of the comparison). Best of 7 on a shared
Xeon 2.8 GHz; Rust is built with plain `rustc -O`, the same target CPU pycc-next assumes:

| program | CPython 3.13 + numpy | pycc-next | Rust (`-O`) | vs CPython | vs Rust |
|---|---|---|---|---|---|
| stencil2d | 4.808s | 0.011s | 0.004s | **423.6x faster** | 2.6x slower |
| prefix_sum_np | 3.823s | 0.048s | 0.015s | **79.5x faster** | 3.2x slower |

My targets were **at least 30x faster than CPython** and **within 3x of Rust**. The
first is beaten on both programs, and stencil2d meets the second. **prefix_sum_np
misses it slightly**, at 3.0 to 3.2x across runs. Its array accesses are a load, a
bounds compare and a store. What's left is the running sum: it's pycc's tagged `int`,
so every add checks for overflow into arbitrary precision. That cost hits every integer
loop, not just numpy ones.

## 🧠 The design: erase the array early

I did not teach the type checker, MIR and codegen a new kind of value. An array stays an
ordinary CPython object, so reference counting, the `--ext` boundary and passing it to
`np.dot` all already work.

A typed array exists as its own type (`ndarray[float64, 2-D]`) for exactly one
compiler pass. That pass:

1. **Infers the dimension count.** `NDArray[np.float64]` doesn't say whether it's 1-D or
   2-D, so I read it off how you use the name: `a[i, j]`, `a[i][j]`, `n, m = a.shape`,
   or `np.zeros((n, m))`. No evidence means 1-D.
2. **Checks every binding.** A parameter, an assignment, a loop variable, a class
   attribute, and a return value must be a `numpy.ndarray` of that dtype and ndim. If not:
   `TypeError: expected numpy.ndarray[float64, 2-D], got an array of dtype int64`.
3. **Rewrites array operations into one node**, then erases the type back to "object".

Codegen turns that node into a read of numpy's array struct (data pointer, shape,
strides). The layout is validated once per process against numpy's own Python
attributes. Any stride layout works: views, transposes, `a[::2]`.

## 🛡️ The part I got wrong first

My first version read a local's shape and data pointer **once per binding** and kept
them. That was fast and also unsafe. Python code called in between can do
`a.resize(1, refcheck=False)`, which frees the buffer, or `a.__setstate__(...)`, which
can change the dtype. The compiled loop then reads freed memory. A review caught it, and
under `PYTHONMALLOC=debug` the old build returned `-1.4568e+144`, the allocator's
freed-memory fill pattern.

The fix keeps the speed. A local's header lives in registers next to a "fresh" bit. After
codegen, a pass walks every call in the function. Any call that could run Python code
(anything but pycc's own number arithmetic and LLVM intrinsics, and yes, a generator's
`yield` counts) clears the bit. The next access re-checks the array and re-reads the
header. The check is inline: exact type, ndim and the canonical dtype pointer, a few
instructions. A loop that calls no Python never re-checks. A global or an attribute,
which anyone can rebind, is checked at every use.

## ⚖️ Semantics I had to pick

- **Elements are Python numbers.** `a[0]` on a `float64` array is a `float`, not
  `np.float64`. So `[a[0]]` prints `[1.0]` where CPython prints
  `[np.float64(1.0)]`, and `int64` math doesn't wrap. Documented in
  `docs/DEVIATIONS.md`.
- **Errors match numpy's text exactly.**
  `IndexError: index 5 is out of bounds for axis 0 with size 5`, negative indices
  included, and `ValueError: assignment destination is read-only`.
- **`a[True]` is a compile error.** numpy reads it as a mask; a native load would
  silently mean `a[1]`.
- **Slices and methods still go through CPython:** `a[1:3]`, `a.sum()`, `np.dot`.

## 🔜 Next

A raw-machine-word path for integer accumulators like prefix_sum_np's. After that:
3-D arrays, `float32`/`int32`, and slices compiled natively.
