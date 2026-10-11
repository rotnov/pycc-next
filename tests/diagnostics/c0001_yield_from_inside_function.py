from typing import Iterator


def f() -> Iterator[int]:
    x = yield from [1, 2]
