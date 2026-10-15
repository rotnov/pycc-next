"""Functions returning tuples of CPython objects, unpacked by their
callers (non-strict mode): compiled code returns the values without a
tuple. Results, evaluation order, exceptions and identities are CPython's;
every case prints what CPython prints."""

from typing import Any


def _note(label: str, value: Any) -> Any:
    print("eval", label)
    return value


def _leaf(src: str, pos: int) -> tuple[int, Any]:
    if pos >= len(src):
        return pos, None
    if src[pos] == "t":
        return pos + 4, True
    if src[pos] == "n":
        return pos + 1, 10**30 + pos
    if src[pos] == "k":
        return pos + 1, {"k": pos}["k"]
    return pos + 1, src[pos]


def _mid(src: str, pos: int) -> tuple[int, Any]:
    if src.startswith("x", pos):
        return _leaf(src, pos + 1)
    return _leaf(src, pos)


def _ordered(a: Any, b: Any) -> tuple[Any, Any, Any]:
    return _note("first", a), _note("second", b), _note("third", a)


def _raising(d: Any, key: Any) -> tuple[int, Any]:
    try:
        return 1, d[key]
    except KeyError:
        return 2, "missing"


def _propagates(d: Any, key: Any) -> tuple[int, Any]:
    return 3, d[key]


def _with_finally(flag: bool) -> tuple[int, Any]:
    try:
        if flag:
            return 1, "body"
    finally:
        print("finally")
    return 2, "after"


def _recursive(n: int) -> tuple[int, Any]:
    if n == 0:
        return 0, []
    count, items = _recursive(n - 1)
    items.append(n)
    return count + 1, items


def _same(o: Any) -> tuple[Any, Any]:
    return o, o


def scan(src: str) -> list[Any]:
    pos = 0
    out: list[Any] = []
    while pos < len(src):
        pos, value = _mid(src, pos)
        out.append(value)
    return out


def main():
    print(scan("abtrue nkxy"))
    pos, value = _leaf("", 0)
    print(pos, value)
    a, b, c = _ordered("A", "B")
    print(a, b, c)
    print(_raising({1: "one"}, 1), _raising({}, 1))
    code, text = _raising({}, 5)
    print(code, text)
    try:
        code, text = _propagates({}, "gone")
    except KeyError as e:
        print("KeyError", e, code, text)
    print(_with_finally(True), _with_finally(False))
    flag, where = _with_finally(True)
    print(flag, where)
    count, items = _recursive(4)
    print(count, items)
    kept = _mid("xt", 0)
    print(kept, type(kept).__name__)
    marker = object()
    first, second = _same(marker)
    print(first is marker, second is marker)


main()
