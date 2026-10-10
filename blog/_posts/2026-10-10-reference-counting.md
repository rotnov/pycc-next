---
title: "Every object pycc-next ever made was immortal. Not anymore."
---

Until today a compiled program **never freed a list or a class instance**. Not one. And a loop
that compared strings leaked **48 bytes per iteration**, so a 3M-iteration loop quietly ate
~140 MB. For a batch script that exits, you shrug. For an `--ext` module living inside
somebody's long-running Python process, that is a bug. 🩸

Now `str`, `list[T]` and class instances are **reference counted**, and the same loops run in
constant memory.

## What was leaking 🔍

Strings already had a refcount. The leak was in the *consumers*: an expression that uses a
string without storing it never gave its reference back.

```python
for i in range(n):
    if f"x{i}y" == "x5y":   # both strings leaked, every iteration
        total += 1
```

Comparison operands, `+` operands, `str * n`, `if s:`, a discarded call result, a dict
lookup key, an `in` needle, `or`/`and` operands that lose: each leaked its temporary. Lists
and instances had no count at all, by design ("leak-only", fine for a first release).

## The design ⚙️

One rule for all three kinds:

- **Every value is either borrowed or fresh.** A name, an attribute of a borrowed object, an
  element of a borrowed list: borrowed. Everything else (literals, displays, call results,
  constructors) is born owning one reference.
- **Every place that holds a value owns one reference**: locals, parameters, globals, instance
  slots, list elements. Storing a borrowed value takes a reference first; overwriting releases
  the old one; a frame releases its slots on every exit, exceptions included.
- **A consumer that doesn't store a fresh value releases it.** That is the leak fix.
- `f().x` and `make_list()[0]`: the read takes its own reference *before* the fresh base is
  released, so the attribute can't die with its owner.
- A method receiver like `s.items.append(s.swap())` is held across the arguments, because
  `swap()` may rebind `s.items` (CPython keeps the receiver alive too, and so do we now).

Lists and instances keep the count at the same offset (byte 32), so the increment and
decrement are **inlined**: a load, an add, a store, and on the decrement a predictable branch
to a cold `free` call. Freeing walks a **worklist**, not recursion: a million-node linked chain
is released in constant stack.

**`--ext`:** a CPython carrier object now owns a reference to the native instance it wraps.
Instances crossing into a compiled function, coming back out, getting copied with
`copy.copy` or stored through a property setter all keep exact counts. A Python loop calling
compiled functions 300 times leaves the live `str`/`list`/instance counters **exactly where they
started**.

## What it costs 💸

Fresh A/B on the same shared box: the commit before vs. after, same programs, median of 9:

| program | instructions before | after | change | median time before | after |
|---|---|---|---|---|---|
| fib | 180.2M | 180.2M | 0.0% | 0.0175s | 0.0175s |
| mandelbrot | 437.2M | 437.2M | 0.0% | 0.0353s | 0.0345s |
| nbody | 1174.4M | 1181.8M | **+0.6%** | 0.1148s | 0.1148s |
| sieve | 710.1M | 710.1M | 0.0% | 0.0662s | 0.0656s |
| spectral_norm | 523.6M | 530.1M | **+1.2%** | 0.0350s | 0.0344s |

Instruction counts are callgrind's, the steady number on this box (load 3 to 6, so single
timings swing by 20% either way and I won't quote them as wins or losses).

The first version cost nbody **+6.4%** instructions: `c = bodies[j]` in the innermost loop paid
an inline increment for the new body and a decrement for the old one, every trip. So a local
that is only ever bound from an element of a parameter list that the function never rebinds or
mutates (and that calls nothing that could) now **borrows**: no count traffic at all. That took
nbody back to +0.6%. spectral_norm's +1.2% is the owned `list[float]` parameters and returned
lists of `mul_av`, now released instead of leaked.

## What it doesn't do (yet) ⚠️

- **Cycles leak.** `a.other = b; b.other = a` keeps both alive forever. CPython's cycle
  collector would free them; there is no collector here. Documented in `docs/DEVIATIONS.md`.
- **`__del__` is never called** on a compiled class.
- **Dicts and sets** are still leak-only.
- A temporary that is live when an exception unwinds can leak once per exception.

Tests: leak probes that run each loop body for 20k and 220k iterations and require the peak RSS
not to grow, oracle fixtures for aliasing and release order run under valgrind memcheck, and an
`--ext` round trip that checks the runtime's live-object counters.
