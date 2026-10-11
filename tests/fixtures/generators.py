from typing import Generator, Iterator


def count(n: int) -> Iterator[int]:
    i = 0
    while i < n:
        yield i
        i += 1


def words(text: str) -> Iterator[str]:
    for w in text.split():
        if w:
            yield w.upper()


def pairs(xs: list[int]) -> Iterator[tuple[int, int]]:
    for i in range(len(xs)):
        for j in range(i + 1, len(xs)):
            yield (xs[i], xs[j])


def guarded(n: int) -> Generator[int, None, None]:
    try:
        for i in range(n):
            yield i * 10
    finally:
        print("cleanup", n)


def fails(n: int) -> Iterator[int]:
    yield n
    yield 10 // (n - n)


def lists() -> Iterator[list[int]]:
    acc: list[int] = []
    for i in range(3):
        acc = acc + [i]
        yield acc


def early(n: int) -> Iterator[int]:
    for i in range(n):
        if i == 2:
            return
        yield i


def total() -> int:
    s = 0
    for v in count(5):
        s += v
    return s


def inner(tag: str) -> Iterator[int]:
    try:
        yield 1
        yield 2
    finally:
        print("inner done", tag)


def outer() -> Iterator[int]:
    try:
        yield from inner("o")
        yield 3
    finally:
        print("outer done")


def chain(a: list[int], b: list[int]) -> Iterator[int]:
    yield from a
    yield from b


def boom() -> None:
    try:
        for v in inner("b"):
            print("v", v)
            raise ValueError("x")
    finally:
        print("boom finally")


def abandoned() -> None:
    g = outer()
    print("first", next(g))
    print("leaving")


class Bag:
    def __init__(self, items: list[int]) -> None:
        self.items = items

    def evens(self) -> Iterator[int]:
        for x in self.items:
            if x % 2 == 0:
                yield x


def _private(xs: list[int], t: int):
    for x in xs:
        if x > t:
            yield x * 2


for v in count(3):
    print(v)
print(list(count(4)), sum(count(5)), max(count(6)), sorted(words("b a c")))
for w in words("hello big world"):
    print(w, len(w))
for a, b in pairs([1, 2, 3]):
    print(a, b)
for v in guarded(3):
    print("got", v)
for v in guarded(5):
    print("got", v)
    if v == 20:
        break
print("after break")
g = guarded(2)
print(next(g))
print(next(g))
print(next(g, -1))
print(next(g, -2))
h = count(2)
print(next(h), next(h))
try:
    next(h)
except StopIteration:
    print("stop")
print(list(lists()))
print(list(early(5)), total())
print(any(count(5)), all(count(3)))
for v in outer():
    print(v)
    if v == 1:
        break
print(list(outer()), list(chain([1, 2], [3])))
try:
    boom()
except ValueError as e:
    print("caught", e)
abandoned()
bag = Bag([1, 2, 3, 4])
print(list(bag.evens()), [v + 1 for v in bag.evens()])
print(list(_private([1, 5, 2, 8], 3)), next(_private([1], 0)), next(_private([1], 5), -1))
it = fails(3)
print(next(it))
try:
    print(next(it))
except ZeroDivisionError as e:
    print("caught", e)
print(next(it, 99))
for v in fails(4):
    print("v", v)
