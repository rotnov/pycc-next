---
title: "I benchmarked my compiler against Rust. It lost. Here's the autopsy."
---

I promised I'd post the numbers that make me look bad. Here they are.

Same algorithm, three ways: CPython 3.13, pycc-next, and hand-written Rust (`rustc -O`).
Best of 3, wall clock, process start included.

| program | CPython | pycc-next | Rust | vs CPython | vs Rust |
|---|---|---|---|---|---|
| fib(32) | 0.261s | 0.100s | 0.007s | **2.6x faster** | 13.9x slower 😬 |
| mandelbrot 600x400 | 1.313s | 0.128s | 0.038s | **10.2x faster** | 3.4x slower |

10x over CPython on float-heavy loops is nice. **"Rust-like speed" it is not.** Yet.

## The autopsy 🔬

I opened the machine code for `fib`. Every `n < 2` and `n - 1` is a **call into the
runtime** (`pycc_rt_int_cmp`, `pycc_rt_int_sub`). Each one checks whether the integer is a
small tagged int or a bigint, then does a one-cycle operation. One cycle of work, ~10 of
overhead.

On top of that:

- 🔁 recursive calls go through a **function pointer**, not a direct `call`
- 🚨 after every call, another call asks "did an exception happen?"

Rust does none of that: `n < 2` is one `cmp`.

## Why it's like this 🤔

Python `int` has no overflow. To match CPython exactly, `2**64` must just work. pycc uses
63-bit tagged small ints and promotes to bigint on overflow. That's correct - but the
fast path should be **inline**: check the tag, do the add with an overflow flag, and only
call the runtime when it actually overflows (almost never).

## The plan 🛠️

1. Inline tagged-int fast paths with `llvm.sadd.with.overflow` - runtime call only on overflow.
2. Direct calls to module functions the module never reassigns.
3. Exception check as one inline load, not a call.

Then I'll re-run this exact table. Same programs, same machine, no cherry-picking.

Also on the list: `sieve`, `spectral_norm` and `nbody` don't compile yet - `list[float]`,
`list[MyClass]` and `[x] * n` aren't supported. That's the next fix after loops learn
`break` and `continue` (currently in progress - yes, really).

Building in public means showing the ugly parts. This is one. 🚀
