"""The f-string `=` debug specifier: the source text, then the value's
`repr` (or its conversion or format spec), as CPython renders it."""

n = 42
name = "pycc"
ratio = 2 / 3
xs = [1, 2]
print(f"{n=}", f"{name=}", f"{ratio=}", f"{xs=}")
print(f"{n = }", f"{ name =}", f"{n*2=}", f"{len(xs) + 1 = }")
print(f"{name=!s}", f"{ratio=:.2f}", f"{n=:>6}", f"{name=!r:>8}")
print(f"[{n=}] and {ratio=} done")
