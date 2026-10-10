"""Arbitrary-precision `int` values crossing the `--ext` boundary."""


def factorial(n: int) -> int:
    result = 1
    for i in range(2, n + 1):
        result *= i
    return result


def fib(n: int) -> int:
    a = 0
    b = 1
    for _ in range(n):
        a, b = b, a + b
    return a


def scale(x: int, k: int) -> int:
    return x * k


def floor_pair(a: int, b: int) -> tuple[int, int]:
    return divmod(a, b)


def shifted(x: int, n: int) -> int:
    return (x << n) >> 1


def mixed(x: int, f: float) -> float:
    return x / 3 + f


def is_huge(x: int) -> bool:
    return x > 2**64
