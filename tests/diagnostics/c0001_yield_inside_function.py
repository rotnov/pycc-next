from typing import Iterator


def f() -> Iterator[int]:
    xs = [(yield i) for i in range(3)]
