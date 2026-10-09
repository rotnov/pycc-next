def f() -> int:
    d = {}
    d[missing] = 1
    return len(d)


print(f())
