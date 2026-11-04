def f(**f: int) -> int:
    return f["x"]


def h(x: int, h: int = 2) -> int:
    return x + h


def s(*s: int) -> int:
    return len(s)


def k(x: int, *, k: int = 5) -> int:
    return x * k
