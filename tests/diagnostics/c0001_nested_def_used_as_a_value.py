def outer() -> int:
    def inner() -> int:
        return 1
    f = inner
    return f()
