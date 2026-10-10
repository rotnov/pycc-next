"""Large `int` arguments reaching machine-word positions in an `--ext`
module: each must raise a Python exception, never abort the host."""


def at(i: int) -> int:
    xs = [1, 2, 3]
    return xs[i]


def push(v: int) -> int:
    xs = [1]
    xs.append(v)
    return len(xs)


def head(n: int) -> int:
    xs = [1, 2, 3]
    return len(xs[:n])


def rep(n: int) -> str:
    return "ab" * n


def buf_at(b: memoryview, i: int) -> float:
    return b[i]


def divide(a: int, b: int) -> int:
    return a // b
