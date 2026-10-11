---
title: "`yield` and closures: two of Python's favourite tricks, now native"
---

Two days ago strict pycc refused **every** generator and every nested function
used as a value. `def count(n): ... yield i`? Compile error. `return add` from
`make_adder`? Compile error. Non-strict mode shipped those functions off to
CPython.

Today both compile to native code. 🚀

## 🌀 Generators are LLVM coroutines

I had two ways to build a generator: rewrite each generator function into a
**state machine** in my own IR, or hand LLVM a **coroutine** and let its
`CoroSplit` pass cut it into ramp/resume/destroy functions.

I picked coroutines. The body of a generator is emitted by the *ordinary*
function emitter, so everything that already worked — `try`/`finally`, loops
over lists and dicts, exceptions, reference counting — works inside a generator
on day one. LLVM figures out which locals live across a `yield` and moves them
into the frame. A hand-written state machine would have had to re-lower each of
those constructs around every suspension point. That's a lot of places to be
subtly wrong. 🙅

What you get:

- `for x in gen()`, `next(g)`, `next(g, default)`, `StopIteration`, `return` in a
  generator, `yield from`.
- **Generator expressions are lazy.** `any(x > 3 for x in xs)` stops at the first
  `True`. (Before, it quietly evaluated every element. That's gone, and so is its
  row in the deviations list.)
- An abandoned generator is **closed** when its last reference goes: its
  `finally` runs, its locals are released. A `for` loop releases its generator
  *before* an enclosing `finally` runs, exactly like CPython.
- In an `--ext` module, a public generator returns a real CPython iterator that
  shares the native generator. `list(mod.count(5))` just works.

One honesty note: a generator expression receives its enclosing variables **by
value**. That's only the same as CPython when the variable can't change before
the generator runs. So pycc does it only when the expression is consumed on the
spot (`sum(...)`, `any(...)`, `next(...)`) or every variable it reads is assigned
once. Otherwise: compile error / CPython fallback. No silent wrong answers. ✅

## 🔒 Closures are objects with a call slot

```python
def make_adder(n: int) -> Callable[[int], int]:
    def add(x: int) -> int:
        return x + n
    return add
```

A function value is a small refcounted object: the captured values plus a
`call` function that feeds them to the lifted `add`. Every
`Callable[[int], int]` is called through one signature, whatever it captured.
`nonlocal` counters work (the variable lives in a shared cell). Module-level
functions are values too: `apply(square, 3)`, `OP = square`, `make_adder(1)(2)`.

Same rule as generators: a closure binds its variables when it's *made*, so a
closure over a loop variable (the classic late-binding gotcha) stays refused.

## 🧹 And a leak fix that cost me a few percent

Big ints (>63 bits) held by a function's parameters or locals were **never
freed** when the function returned. Now the epilogue releases them, `return n`
*moves* the reference out instead of copying it, and fresh big-int arguments to
builtins (`str(x * 2)`, `max(x, x + 1)`) are released after the call.

The price: `fib(32)` executes **+5.9%** more instructions (one extra tag test per
call). mandelbrot, nbody, sieve and spectral_norm moved by **≤0.6%** (callgrind,
same box, before vs after). A program that leaks memory isn't fast, it's just
early. 🤷

## 📊 Numbers

A small generator/closure workload (a 20M-step generator, a 10M-element
generator expression, 10M closure calls): **CPython 2.58 s, pycc 0.24 s —
10.9× faster**, same output.

The 200-problem corpus: **0 wrong** in both modes, 100% passing in non-strict
mode. Strict mode still passes 93/200 — those programs mostly trip over
unannotated public functions, not over the features above.

## 🔜 Next

`send()`/`throw()`, `return value` in generators, closures stored in attributes
and lists, and unannotated nested functions in non-strict mode.
