---
title: "My type errors all said 'line 1'. Now they point at the bug."
---

Embarrassing confession: until today every type error pycc-next reported looked like this.

```
error[T0021]: argument 1 of `fib` expects `int`, got `str`
 --> fib.py:1:1
  |
1 | def fib(n: int) -> int:
  | ^ argument 1 of `fib` expects `int`, got `str`
```

Right message. **Wrong place.** Line 1, column 1, and the caret label just repeated the headline. 🙈

## 🎯 Now

```
error[T0021]: argument 1 of `fib` expects `int`, got `str`
 --> fib.py:6:11
  |
6 | print(fib("5"))
  |           ^^^ expected `int`, found `str`
```

Same for every type error: an unbound read underlines the name (`may be unbound here`),
a bad assignment underlines the value, a missing return underlines the `-> int`, a
heterogeneous list underlines the odd element. `--error-format json` carries the same
line, column and length.

## 🔧 How, without touching 10,000 match arms

The type checker works on HIR, and HIR nodes have **no spans**. Adding a span field to
every node would have meant editing roughly ten thousand construction and match sites.

So positions are recovered after the fact. When a check fails, the driver re-parses the
file, and `pycc_hir::locate` pairs every HIR node with the AST node it came from: same
kind, same name, same order within its statement. Where lowering rewrote code (`x += 1`
becomes `x = x + 1`, `assert` becomes an `if`, `a, b = t` binds a temporary) the AST side
models the rewrite. When a pairing is ambiguous it is **skipped, not guessed**, and the
diagnostic falls back to the enclosing statement, then the function header.

The check then runs again with those spans, and each error is placed at the innermost
expression whose check failed. A program that type-checks pays **nothing**: no second
parse, no extra walk.

## 📊 Proof

A new test compiles 26 broken programs, one per common error code plus methods, lambdas,
f-strings and `elif` arms, and asserts the reported `line:col` and length equal the
offending expression's. All 26 pass. 90 diagnostic snapshots moved off `1:1`; each one
was reviewed.

## 🚧 Still at line 1

Whole-program pre-checks (incompatible redefinitions, attribute slots no assignment
types) and the post-check monomorphization phase have no single expression to blame yet.
