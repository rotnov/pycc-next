---
title: "200/200 corpus programs now run. Most of their code still runs in CPython, and I'll show you exactly how much."
---

Yesterday's non-strict mode compiled untyped Python... until it hit one construct I
haven't implemented. One `lambda` anywhere, and the whole program was a compile error.
On the 200-program competitive-programming corpus that meant **2 of 200** built.

Today: **per-function fallback.** 🧩

## 🔧 What happens now

In `--non-strict` mode, a top-level function, class or statement pycc can't compile yet
is kept as **Python source**. At module init the compiled module hands it to CPython's own
`exec`, in the module's namespace, at the exact position it had:

```console
$ pycc build --non-strict --report-fallbacks m.py -o m
fallback: function `describe` at m.py:11-14 runs in CPython: C0001 expression kind not supported yet: a `lambda`
fallback: statement at m.py:48 runs in CPython: C0001 expression kind not supported yet: a generator expression
```

Everything else stays native. The two sides talk through plain Python:

- compiled code calls a fallback function as a **CPython object**;
- fallback code calls compiled functions as the module's **published callables**
  (an embedded `__main__` now publishes them too, like an `--ext` module);
- a module global that fallback code reads or binds lives in the **module dict**, and
  compiled references to it become dict lookups, so `global counter; counter += 1` in a
  lambda-using function is seen by the compiled `print(counter)` right after;
- exceptions cross both ways with CPython's type and message, and fallback tracebacks
  keep their real line numbers (the source is padded before `compile`).

Strict mode is untouched: same diagnostics, same refusals.

## 📊 The numbers (fresh run, `python3 bench/corpus.py -j 3 --flags=--non-strict`)

| | before | after |
|---|---|---|
| pass | **2/200** | **200/200** |
| wrong output | 0 | **0** |
| compile errors | 198 | 0 |

Now the part that keeps me honest: **881 of 1,191 top-level statements (74%) ran in
CPython**, and **102 programs ran entirely in CPython**. Only the same 2 programs as
before have no fallback at all. The corpus is scripts that read `input()` at module
level, and `input` is still a gap, so a lot of them are "compile a preamble, exec the
rest". That's not a speed story yet. It's a **correctness floor**: every program now
runs with CPython's semantics, and every fallback line is a named, countable gap.

The top reasons, straight from `--report-fallbacks` over the corpus: builtin `input`
(184 statements), `list` ⊕ object arithmetic (97), int/object inference conflicts (71),
`lambda` (49), subscript assignment targets (42), `map` (38). That's the to-do list,
ranked.

## 🐍 Also: no more pinned CPython

The embedded interpreter used to be pinned to the 3.14 line (tests to exactly 3.14.7).
Now everything that needs CPython (`--ext` headers, embedded builds, the oracle in the
tests) uses `PYCC_PYTHON` or `python3`: **any CPython ≥ 3.13**, matching the 3.13
oracle. CI now runs the ~500 hosted tests that used to sit behind `#[ignore]`.

## ⏭️ Next

Shrink that 74%: `input`, list/object arithmetic and subscript targets alone account for
over a third of the fallbacks. Each one that lands moves statements from the CPython
column to the native one, and the corpus runner will say so.
