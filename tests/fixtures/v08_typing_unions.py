"""v0.8 accept 4: `typing.Optional[X]` and `typing.Union[...]` are the unions
`X | None` and `X | ...` they spell."""

from typing import Optional, Union


def first_index(items: list[int], wanted: int) -> Optional[int]:
    for i, item in enumerate(items):
        if item == wanted:
            return i
    return None


def label(name: Optional[str]) -> str:
    if name is None:
        return "<none>"
    return name.upper()


def half(n: Union[int, float]) -> float:
    return n / 2


def or_zero(n: Union[int, None]) -> int:
    if n is not None:
        return n
    return 0


def same(s: Union[str]) -> str:
    return s + s


def maybe_len(s: Union[None, str]) -> int:
    if s is not None:
        return len(s)
    return -1


print(first_index([4, 5, 6], 6), first_index([], 1), first_index([7], 7))
print(label("abc"), label(None))
print(half(3), half(2.5))
print(or_zero(None), or_zero(41) + 1)
print(same("ab"), maybe_len(None), maybe_len("four"))
values: list[Optional[int]] = [1, None, 3]
total = 0
for v in values:
    if v is not None:
        total += v
print(total)
print(first_index([1, 2, 3], 9))
print(label(None)[10])
