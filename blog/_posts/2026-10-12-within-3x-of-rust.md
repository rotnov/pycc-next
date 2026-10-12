---
title: "Every benchmark is now within 3x of Rust. Mostly by proving checks can't fire."
---

v0.1 shipped with **fib 3.8x, nbody 4.2x, spectral_norm 3.5x, sieve 3.1x slower than Rust**.
The v0.2 goal was "every program within 3x". Today all five are under it, with the exact same
output as CPython:

| program | v0.1 (before) | today | Rust | vs CPython | vs Rust before -> after |
|---|---|---|---|---|---|
| fib(32) | 0.025s | **0.015s** | 0.008s | **24x faster** | 2.9x -> **1.8x** |
| mandelbrot | 0.060s | 0.061s | 0.055s | **30x faster** | 1.0x -> **1.1x** |
| nbody | 0.143s | **0.084s** | 0.034s | **31x faster** | 3.4x -> **2.4x** |
| sieve | 0.063s | **0.057s** | 0.028s | **31x faster** | 2.3x -> **2.1x** |
| spectral_norm | 0.037s | **0.021s** | 0.012s | **56x faster** | 3.2x -> **1.8x** |

`python3 bench/run.py --runs 11`, best of 11, on a shared 4-core Xeon with a **load average of
8 to 15**. Wall times on that box wobble by 20% (a best-of-7 run during heavier load had nbody
at **2.7x**), so every decision below was made on instruction counts (callgrind), which don't:

| program | before | after | Rust |
|---|---|---|---|
| fib | 190.8M | **137.9M** | 69.5M |
| mandelbrot | 437.6M | **334.8M** | 220.7M |
| nbody | 1184.5M | **640.2M** | 295.4M |
| sieve | 710.1M | **593.7M** | 164.7M |
| spectral_norm | 526.9M | **248.2M** | 106.4M |

## Two changes to the Rust side ⚖️

I changed the yardstick, so I'm putting this first.

1. **No more `-C target-cpu=native` for Rust.** pycc-next compiles for plain x86-64, so native
   flags measured the flag, not the compiler. It barely mattered (within noise).
2. **The Rust twins black-box every input**, which the bench rules already required. nbody's
   Rust version didn't: rustc saw the literal 5-body list and **unrolled and vectorized every
   pair** — something no compiler can do for a Python list whose length is a run-time value.
   With the old twin, nbody is still **3.2x** behind Rust (0.092s vs 0.029s). I'm counting the
   new number, and I'm telling you the old one.

## What was slow 🔍

Not the math. The **checks around the math**, each one cheap and each one repeated millions of
times:

- every `b.x` tested an "is this attribute assigned yet?" flag (CPython raises
  `AttributeError` for a read before `__init__` assigns it) — **15 tests per nbody pair**;
- every call was followed by "did that raise?";
- every `int` operation tested its operands' tags, then the next operation tested the result's
  tag again;
- `x += 1` tested whether the old `x` was a heap bigint that needs freeing;
- outer `for` loops counted with tagged, possibly-bigint ints.

## What I changed 🔧

1. **Assigned-attribute proof.** If every constructor of a class assigns `self.x` before
   `self` can escape (no call that gets `self`, no read of `self`), then no code can ever see
   `x` unassigned. Those reads skip the flag test and later stores skip the flag write.
   Subclasses count: if one of them has an `__init__` that skips `x`, the proof fails for
   everyone. nbody: **-20% instructions**.
2. **Functions that can't raise.** `fib` only does `<`, `-`, `+` on ints (arbitrary
   precision, no overflow) and calls itself. Computed as a fixed point, so recursion
   qualifies. Callers no longer check for an exception after the call. fib: **-28%**.
3. **One fast path per integer expression.** `(i + j) * (i + j + 1) // 2 + i + 1` now tests
   the tags of `i` and `j` once, computes on machine integers with overflow checks, and
   re-tags once. Any bigint or overflow falls back to the old per-operator code. spectral_norm:
   **-32%**.
4. **`x += y` frees the old value only on the slow path.** If the fast path ran, the old
   value was a small int, which owns nothing. sieve **-16%**, mandelbrot **-22%**.
5. **Raw loop counters for outer loops too**, not only innermost ones. nbody **-18%**.
6. **A function defined before any module code runs** skips the "called before its `def`"
   check.

## The `** 0.5` story, part 2 🧮

CPython computes `d2 ** 0.5` with libm `pow`, and glibc's `pow(x, 0.5)` differs from
`sqrt(x)` in the last bit for about 1 in 1,000 inputs. Last time I proved "`pow` must return
`sqrt(x)` here" with a floating-point residual: ~30 instructions per call.

This time the residual is **exact integer arithmetic**: with `s = S * 2^e` and
`x = X * 2^f`, the scaled residual `X * 2^k - S^2` is an integer smaller than `2^53`, so
wrapping 64-bit math gives it exactly: `(x_bits << k) - S*S`. About 20 instructions, and it
now covers every positive normal `x` instead of a 2^-900..2^1000 window. Checked against glibc
on **620M inputs, including 19M where `pow != sqrt`: 0 mismatches**.

## The honest part 📉

- **mandelbrot didn't move** (1.0-1.1x): it was already at Rust speed.
- **sieve's wall time barely moved** although its instruction count dropped 16%: it is limited by
  memory, and its `while` loop still tests `j`'s tag three times per step.
- **nbody's remaining gap is structural**: Rust keeps a `Vec<struct>` and does two pairs per
  SIMD instruction; we chase a pointer per body and still pay ~20 instructions per pair to stay
  bit-identical to CPython's `pow`.
- One test in the suite (`nbody` must be 20x faster than CPython on a different program) fails
  on this machine under load; its instruction count is identical before and after this work.

## What's next 🚀

numpy arrays as native types, and the next real-world corpus milestone.
