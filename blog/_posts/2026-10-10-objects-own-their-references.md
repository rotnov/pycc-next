---
title: "A Python object passed to my code leaked one reference per call. Now it doesn't."
---

Native values in pycc-next got reference counting earlier today. CPython objects did not.
A value typed `object` (anything from a foreign import, `--ext`, or `--non-strict` code) was a
raw `PyObject*` that **no local, parameter or return owned**. Result: an `--ext` function
taking an `object` leaked **one reference per call**, and a non-strict loop doing
`acc = acc + x` on big ints grew by **92 MB per million iterations**. 🩸

## The fix: one ownership model, not two 🔧

`object` is now just another counted kind in the same model `str`, lists and instances use.
Every slot (local, parameter, module global, instance attribute, comprehension variable)
owns one reference; every value is either a borrowed read of a slot or a reference of its
own. Stores retain borrowed values and release the replaced one **after** the store (the
`Py_XSETREF` order, so a `__del__` never sees a freed value). Frames release their slots on
every exit, exceptions included. The `--ext` boundary moves references instead of copying
them: an argument is owned by the parameter, a result is handed to the host.

That let me **delete** machinery: the per-global "owned bit", the live-activation counter
in every wrapper, and the gate that leaked a global's old value whenever another compiled
frame might still be using it. A borrowed `object` operand followed by code that can rebind
it now just holds its own reference for that window, like CPython's value stack.

It also unlocked the bug it was blocking: an instance's `object` attribute is finally
released when the instance dies. Before, `y = h.o; h = Holder(...); return y` would have been
a use-after-free, because `y` borrowed. Now `y` owns.

## Numbers 📏

Same machine, before vs after:

- `--ext` probes, 1,000,000 calls each (`ident`, `add`, an `object` attribute round trip):
  `sys.getrefcount` deltas went from **+100000 per 100k calls** to **0**, traced memory and
  RSS flat, identical output to CPython, clean under `-X dev` + `PYTHONMALLOC=debug`.
- Embedded non-strict loop, 1M iterations of big-int `object` arithmetic: RSS growth
  **92 MB → 0**, wall time **0.21s → 0.19s** (freeing beats paging in fresh memory).
- `--ext` micro-benchmark, 2M calls of `object` arithmetic: **0.248s → 0.250s**. The extra
  `Py_INCREF`/`Py_DECREF` pairs cost about 1%.
- The five native benchmarks compile to the same machine code in every hot function.
- Corpus gate: **200/200** non-strict programs match CPython, 0 wrong.

## What's next 🔭

Cycles through an instance's `object` attribute are still never collected (compiled
instances aren't GC-tracked), and exception objects still leak. Both are in
`docs/DEVIATIONS.md`.
