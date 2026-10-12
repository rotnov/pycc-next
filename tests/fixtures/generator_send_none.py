from typing import Generator


def nothing() -> Generator[int, None, int]:
    got = yield 1
    print(got)
    got = yield 2
    print(got)
    return 7


def taker() -> Generator[int, int, None]:
    yield 0
    yield 1


def counted() -> Generator[int, None, int]:
    yield 1
    return 42


def drain(g: Generator[int, None, int]) -> None:
    next(g)
    next(g)


def main() -> None:
    # A generator taking `None` values takes `send(None)` everywhere.
    n = nothing()
    print(n.send(None))
    print(n.send(None))
    try:
        n.send(None)
    except StopIteration as e:
        print("returned", e.value)

    # One taking `int` values is primed with `next()` (`send(None)` is a
    # compile-time error); `send(v)` before that is CPython's `TypeError`.
    t = taker()
    try:
        t.send(3)
    except TypeError as e:
        print("TypeError:", e)
    print(next(t))
    print(t.send(5))

    # A `StopIteration` raised in a helper by a generator returning the
    # same type as the ones the `try` body resumes keeps its value.
    first = counted()
    second = counted()
    try:
        next(first)
        drain(second)
    except StopIteration as e:
        print("helper returned", e.value + 1)


main()
