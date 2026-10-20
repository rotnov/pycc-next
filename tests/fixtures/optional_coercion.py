"""A CPython object returned from, or passed to, a compiled function as `T | None`
(`tests/optional_coercion.rs`)."""

from typing import Optional


def lookup(table, key: str) -> Optional[int]:
    return table.get(key)


def name_of(table, key: str) -> str | None:
    return table.get(key)


def ratio(table, key: str) -> float | None:
    return table.get(key)


def flag(table, key: str) -> bool | None:
    return table.get(key)


def twice(n: Optional[int]) -> int:
    return -1 if n is None else n * 2


def via(table, key):
    return twice(table.get(key))


def main() -> None:
    t = {"a": 3, "b": None, "big": 10**30, "s": "text", "f": 2.5, "t": True}
    print(lookup(t, "a"), lookup(t, "b"), lookup(t, "zz"), lookup(t, "big"))
    print(name_of(t, "s"), name_of(t, "b"), ratio(t, "f"), ratio(t, "b"))
    print(flag(t, "t"), flag(t, "b"), via(t, "a"), via(t, "b"))


if __name__ == "__main__":
    main()
