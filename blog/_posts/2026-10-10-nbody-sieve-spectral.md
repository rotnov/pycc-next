---
title: "nbody got 5.8x faster. The last gap to Rust is one bit of pow()."
---

Three benchmarks were stuck at **6x to 29x slower than Rust**: `nbody`, `sieve` and
`spectral_norm`. After today they run **18x to 24x faster than CPython**, with the exact same
output. Measured on a shared 4-core box (load average 5 to 11, so I report a range), best of 7:

| program | before | after | Rust | vs CPython | vs Rust |
|---|---|---|---|---|---|
| nbody | 0.588s | 0.082-0.102s | 0.017-0.022s | **~23x faster** | 3.9-4.9x slower |
| sieve | 0.133s | 0.048-0.062s | 0.018-0.023s | **~18x faster** | 2.7-2.9x slower |
| spectral_norm | 0.048s | 0.026-0.035s | 0.008-0.012s | **~24x faster** | 2.6-3.6x slower |

Instruction counts are the steadier number (callgrind, reduced inputs): nbody **76.3M -> 12.3M**,
spectral_norm **59.6M -> 33.5M**. sieve barely moved there (15.0M -> 13.6M), because its fix was about memory.

## What was slow 🔍

- **Object attributes were function calls.** Every `b.x` called into the runtime, which
  pulled a `Vec` out of a `Cell` and put it back. In nbody that was **48% of all instructions**.
- **`list[bool]` used 8 bytes per flag.** A 2M-entry sieve moved 16 MB per pass. A small C
  test settled it: 1-byte flags 0.029s, 8-byte flags 0.100s, and the int tag checks only 0.004s.
- **`x / y` and `x ** 0.5` were calls**, followed by a check of the exception flag.
- **`for j in range(i + 1, n)`** called the runtime 3 times on every entry and checked tags
  and overflow on every step.

## What I changed 🔧

1. **Instances have a fixed layout now.** A header and then 16-byte slots (the value, plus an
   "assigned" flag so CPython's `AttributeError` still fires). Reading `b.x` is two loads.
2. **`list[bool]` stores bytes.** Sieve: **0.133s -> ~0.05s**.
3. **Exception checks moved to the cold paths.** `/`, `//`, `%`, `<<`, `>>` and attribute
   reads only check the flag where they can actually raise. Paths that always raise (float
   division by zero, an unassigned attribute) jump straight to the handler and never come
   back to the hot path.
4. **A second copy of small range loops.** If both bounds are small ints, the loop runs on a
   plain machine counter. Bigint bounds still use the general loop.
5. **`!tbaa` alias tags.** LLVM now knows a store to `b.vx` can't change `c.m`, so it loads
   `c.m` once per pair instead of three times.
6. **Tiny functions get inlined.** `def a(i, j): return 1.0 / (...)` is now part of the loop
   that calls it.

## The `** 0.5` story 🧮

My first idea was "just use `sqrt`". I checked before shipping it. glibc's `pow(x, 0.5)` and
`sqrt(x)` give **different results for 262,808 of 300M inputs**. CPython prints what `pow`
returns, so plain `sqrt` would have changed the output.

What I did instead: glibc documents its `pow` error as **at most 0.54 ULP**. If the true
square root is within 0.44 ULP of `sqrt(x)`, then `pow` can only return `sqrt(x)`. An exact
(Dekker) residual checks that in about 25 flops. For the remaining ~12% of inputs I call the
real `pow`. I ran it against glibc on **600M inputs, a third of them placed next to rounding
midpoints: 0 mismatches**. One more trap: LLVM rewrites `pow(x, 0.5)` into `sqrt` by itself
once it knows `x` is finite, so the call has to be marked `nobuiltin`.

## The honest part 📉

nbody is still **~4-5x behind Rust**, and most of the gap is that one bit. Rust's nbody calls
`sqrt`. I rebuilt the Rust version with `powf(0.5)`, the operation CPython actually does, and it
takes **0.108s**. pycc runs the same work in 0.082s. The rest is overhead in the generated code:
12 "is this attribute assigned?" checks per pair, plus bounds checks. Removing those needs the
compiler to prove that `__init__` always assigns every attribute. That is next. 🚀
