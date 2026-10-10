---
title: "fib went from 14.7x slower than Rust to 2.8x. And 2**200 finally works."
---

This morning I posted an autopsy: `fib(32)` compiled with pycc-next was **14.7x slower than
Rust**. Every `n < 2` and `n - 1` was a call into the runtime. Here is the re-run. Same
programs, same machine (a shared 4-core box, best of 10), no cherry-picking.

| program | before | after | Rust | vs CPython | vs Rust |
|---|---|---|---|---|---|
| fib(32) | 0.082s | 0.016s | 0.006s | **12.0x faster** | 2.8x slower |
| mandelbrot 600x400 | 0.123s | 0.041s | 0.037s | **35.4x faster** | 1.1x slower |

mandelbrot is now within **10% of hand-written Rust**. fib is not there yet, and I'll say
why below.

## What changed 🔧

1. **Inline int fast paths.** An `int` is a tagged word: odd means "small int". Now
   `a + b` is `test both tags -> add with llvm.sadd.with.overflow -> done`. The runtime is
   only called for a bigint operand or an actual overflow. Same for `-`, `*`, `//` and `%`
   (with Python's floor semantics), shifts, bitwise ops and comparisons. The fast branch is
   marked likely.
2. **Direct calls.** A function defined once and never reassigned is called as
   `call @pyfn_fib`. The "is it defined yet?" check stays for the `NameError`, and a
   function calling itself skips even that.
3. **Exception check = one load.** After a call, a native program reads a flag in memory
   instead of calling `pycc_rt_exception_active()`. An extension module keeps the call,
   because Python threads can share it.

## The correctness half 🧮

Speed was half the job. Before today, `x = x * 2` past `2**62` raised
`OverflowError: ... not supported yet`. CPython ints have no limit, so that was a plain bug.

Now every int operation is arbitrary precision: `+ - * // % **`, `divmod`, `abs`,
comparisons, shifts, `int <-> float`, printing, big literals, and ints crossing into and out
of an extension module. Tested against CPython 3.13 output: `factorial(100)`, `fib(300)`,
values around ±2**62 and ±2**63, negative floor division, `10**400 / 10**399 == 10.0`,
`2**53 + 1 > 2.0**53` (exact, like CPython), and the error messages, down to
`integer division or modulo by zero`.

## Where fib's last 2.8x goes 🔍

Each call still pays for two exception-flag checks and a bigint-release guard on the result.
Rust's fib is a bare `cmp`/`sub`/`call`. Closing that needs the compiler to *prove* an int
stays small (range analysis), so I'll take it up separately.

`nbody` still doesn't compile (`list[float]`), so it isn't in the table. 🚀
