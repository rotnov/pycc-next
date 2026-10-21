"""Membership in set displays of literals."""


def keep(f):
    return f


def kind(k):
    if k in {"list", "set", "frozenset", "generator"}:
        return "seq"
    if k not in {"a", "b"}:
        return "other"
    return "ab"


def num(v):
    return v in {1, 2, 3}, v in {1.5, None, True}


class S(str):
    pass


for _ in range(3):
    print([kind(k) for k in ("list", "a", "zzz", "generator", "b")])
    print([num(v) for v in (1, 4, 1.0, True, None, 1.5, 2)])
print(kind(S("list")), kind(S("q")))
try:
    kind([])
except TypeError as e:
    print("TypeError", e)
