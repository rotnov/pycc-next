---
title: "`g == h` could free `h` halfway through. Now the operand holds its own reference."
---

Earlier today `object` values got the native ownership model: a module global owns its
value, and a *borrowed* read of it costs nothing. One window was left open. A borrowed
operand was pinned only when a **later sibling** could run code -- but the operation that
consumes it runs Python code too.

`g == h` calls `g.__eq__`. If that re-executes the module body (a nested import of the same
compiled module shares its global slots), `h` is rebound and its old value freed. CPython
then calls the reflected `h.__eq__` -- on freed memory. Same story for `d[k]` (the key's
`__hash__` runs before the dict is probed), `k in d`, `d[k] = v`, `g + h` and
`g + box.o`. Under `-X dev` + `PYTHONMALLOC=debug` each of those segfaulted. 💥

## The fix 🔧

CPython's C API borrows its arguments and relies on the caller's reference -- the one on the
value stack. So an operation that runs CPython code now pins each `object` operand that
some *other* activation can drop: a module global, or a read through an attribute or list
element. The pin is a `Py_INCREF` after the read and a `Py_DECREF` after the operation, and
it reuses the existing pin machinery, so every consumer already releases it exactly once.

Not pinned, on purpose:

- **Frame locals.** Only their own frame stores to them; a callee can't rebind the caller's
  locals, and a nested exec gets its own activation.
- **Call arguments and callees.** The shim already packs each argument into a new reference,
  and a borrowed callee is retained by `pycc_ext_obj_call_borrowed`. A method call's
  receiver is pinned only across the lookup -- the bound method holds it for the call.

## Numbers 📏

- New oracle test: 16 operations on rebound globals, in a module body and in a function,
  match CPython line for line under the debug allocator; 5 more calls leave live objects
  and refcounts unchanged. Before the fix the same test segfaults.
- The five native benchmarks: identical machine code in every function.
