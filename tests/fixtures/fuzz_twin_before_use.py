# Found by the differential fuzzer (fuzz/): in an `--ext --non-strict` module
# the CPython twin of a function the boundary cannot carry (a `dict`
# parameter) was defined at the end of the module when no blank line
# followed its `def`, so a statement running in CPython right after it
# raised NameError.
def f(p0: dict[str, int], p2: str) -> dict[str, int]:
    return {p2: len(p0)}
print(f(dict(zip(["a"], [1, 2])), "x"))
print(f({}, "y"))
