import functools
from functools import cache, lru_cache

calls: list[str] = []


@cache
def fib(n: int) -> int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


@lru_cache(maxsize=None)
def double(n: int) -> int:
    calls.append("double " + str(n))
    return n * 2


@lru_cache(maxsize=2)
def square(n: int) -> int:
    calls.append("square " + str(n))
    return n * n


@lru_cache
def join(a: str, b: str, sep: str = "-") -> str:
    calls.append("join " + a + b)
    return a + sep + b


@lru_cache()
def ratio(a: float, b: int) -> float:
    calls.append("ratio")
    return a / b


@functools.cache
def flag(b: bool) -> str:
    calls.append("flag")
    return "yes" if b else "no"


@lru_cache(maxsize=0)
def uncached(n: int) -> int:
    calls.append("uncached " + str(n))
    return n + 1


def main() -> None:
    print(fib(90), fib(10))
    print(double(3), double(3), double(4))
    print(square(1), square(2), square(1), square(3), square(2), square(1))
    print(join("a", "b"), join("a", "b"), join("a", "b", "+"))
    print(ratio(1.5, 2), ratio(1.5, 2), flag(True), flag(True), flag(False))
    print(uncached(1), uncached(1))
    print(calls)
    calls.clear()
    double.cache_clear()
    print(double(3), calls)
    print(ratio(1.0, 0))


main()
