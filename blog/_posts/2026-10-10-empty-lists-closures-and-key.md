---
title: "`[]`, `lambda` and `key=`: the three things every script uses, now native"
---

Look at any competitive-programming script. Somewhere near the top there is a
`seen = set()`, a `graph = [[] for _ in range(n)]`, a `key=lambda p: p[1]`,
and a little `def dfs(v):` nested inside `main`. Strict pycc refused **all four**.

Today they compile. 🛠️

## 🧩 Empty containers get their type from what you do with them

`xs = []` has no element type. CPython doesn't care; a compiler does. pycc now
runs a small **union-find pass** before type checking: every empty `[]`, `{}`,
`set()` and `frozenset()` becomes a variable, and the function body unifies it
with what later happens to it. That covers `.append(x)`, `d[k] = v`, `s.add(x)`,
`for c in text: s.add(c)`, `counts.get(w, 0) + 1`, and being passed to a helper.

The rule that keeps it **honest**: a name bound at several sites with different
types is never guessed. `v = True` in one branch and `v = 1` in the other is a
conflict, so it stays a compile error in strict mode. Picking one type there
would turn a CPython `True` into a printed `1`.

## 🔒 Closures, lambdas and `key=`

- A nested `def` and a lambda that is only ever *called* are lifted to module
  functions. A name they capture becomes a parameter. A `nonlocal` gets a cell.
- Private helpers get their container parameter types from their **call sites**,
  so `def dfs(v)` used with a `list[list[int]]` graph types itself.
- `sorted(xs, key=f, reverse=r)`, `xs.sort(key=...)`, `min`/`max(key=...)`:
  the key is evaluated **exactly once per element**, as CPython does. The result is
  a stable sort on `(key, element)` pairs, with CPython's tie order kept under
  `reverse=True`.

Along the way, sets and dicts joined the shared reference-counting model
(`|=`, `&`, `-`, `^`, `frozenset(...)` with its real `repr`), and `del d[k]` and
`d.pop(k, default)` compile natively.

## 🐛 Bugs fixed on the way

- A security review caught this one: a `dict.get` miss handed a **null sentinel** to the tuple unboxer *before* the
  `KeyError` check. That was a null dereference waiting to happen on
  `dict[str, tuple[int, int]]`. The unboxer now guards it, and an oracle test
  covers a caught and an uncaught `KeyError`.
- A last line printed with `end=" "` and no trailing newline **was never flushed**
  at exit. One corpus program printed nothing at all. 😬 The module now flushes
  stdout when it finishes.

## 📊 Numbers (fresh runs, 200-program corpus)

| | before | after |
|---|---|---|
| strict pass (`bench/corpus.py -j 3`) | **58/200** | **69/200** |
| strict wrong output | 0 | **0** |
| non-strict pass (`--flags=--non-strict`) | 200/200 | **200/200** |
| non-strict wrong output | 0 | **0** |

Non-strict fallback share right now: **776 of 1,191 top-level statements (65%)
still run in CPython**. **45 programs run entirely in CPython**, and **68 have no
fallback at all**. That is better than the 74% I published with per-function
fallback, but it is still most of the code. I won't pretend otherwise.

## 🔭 What's next

The top strict blockers are now:

- an empty list whose element type really can't be seen (6 programs);
- a nested `def` used as a *value*, not just called;
- public functions without return annotations;
- `from x import *`;
- starred targets (`a, *rest = ...`);
- `math.log`/`log2`.

None of them is generator-sized, so I'm going after them next.
