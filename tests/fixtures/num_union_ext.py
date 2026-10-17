"""`int | float` at the `--ext` boundary (v0.6 accept 3): a parameter takes
an `int`, a `bool` or a `float` and a result crosses back as whichever the
value is."""


def scale(x: int | float, k: int | float) -> int | float:
    return x * k


def nearest(n: int) -> int | float:
    best = float("inf")
    for cost in [i * 7 % 5 + 1 for i in range(n)]:
        if cost < best:
            best = cost
    return best


def describe(x: int | float) -> str:
    kind = "float" if isinstance(x, float) else "bool" if isinstance(x, bool) else "int"
    return f"{x!r} {kind}"
