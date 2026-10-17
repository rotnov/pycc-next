# A generator whose send type excludes what the program sends (a fuzzer
# finding): under --non-strict the generator function runs in CPython, which
# receives any value. Oracle: CPython runs this file unchanged.
from typing import Generator


def acc() -> Generator[int]:
    total = 0
    while True:
        x = (yield total)
        total += x


def squares(n: int) -> Generator[int]:
    for i in range(n):
        yield i * i


g = acc()
next(g)
for v in [6, 8, 3]:
    print(g.send(v))
print(list(squares(4)))
