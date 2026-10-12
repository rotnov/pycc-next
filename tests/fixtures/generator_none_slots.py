from typing import Generator


def accumulate() -> Generator[int, int, None]:
    total = 0
    while True:
        total += yield total


def two_values() -> Generator[int, None, int]:
    got = yield 1
    print(got)
    got = yield 2
    print(got)
    return 7


def delegate() -> Generator[int, None, None]:
    result = yield from two_values()
    print("delegated", result)
    nothing = yield from empty()
    print("nothing", nothing)


def empty() -> Generator[int, None, None]:
    if False:
        yield 0


def locals_of_none() -> None:
    x = None
    print(x)
    y: None = None
    print(y)
    z = print("side effect")
    print(z)
    x = z
    print(x is None)


nothing_global = None

g = two_values()
print(next(g))
print(g.send(None))
try:
    g.send(None)
except StopIteration as e:
    print("stop", e.value)

h = accumulate()
print(next(h))
print(h.send(3))
print(h.send(4))
closed = h.close()
print(closed)

for value in delegate():
    print("from", value)

locals_of_none()
print(nothing_global)
nothing_global = None
print(nothing_global)
