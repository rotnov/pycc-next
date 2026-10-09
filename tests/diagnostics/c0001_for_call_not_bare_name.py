xs = [1, 2]


def f() -> int:
    for k in xs[0]():
        return k
    return 0
