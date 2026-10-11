def outer() -> int:
    k = 1

    def inner() -> int:
        return k
    f = inner
    k = 2
    return f()
