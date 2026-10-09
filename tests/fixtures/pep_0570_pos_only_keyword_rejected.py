def f(a: int, /, b: int) -> int:
    return a + b

print(f(a=1, b=2))
