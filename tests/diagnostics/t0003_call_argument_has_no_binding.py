def g(xs: list[int]) -> int:
    return len(xs)


def f() -> int:
    return g([])


print(f())
