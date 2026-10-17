# Found by the differential fuzzer (fuzz/): `case [x, *rest]` bound the
# whole list to `rest`, an or-pattern inside a sequence, mapping or class
# pattern (`[1 | 2]`) required every alternative at once, and a mapping
# pattern whose key the subject lacks raised KeyError instead of failing.
from dataclasses import dataclass


@dataclass
class P:
    x: int
    y: int


rows: list[list[int]] = [[1, 2, 3], [4], [5, 6], []]
for v in rows:
    match v:
        case [x, *rest]:
            print("many", x, rest)
        case _:
            print("empty")
for w in [[1], [2], [3], [1, 2]]:
    match w:
        case [1 | 2]:
            print("one of 12")
        case [a, 2 | 3]:
            print("pair ending 2 or 3", a)
        case _:
            print("no")
items: list[dict[str, int]] = [{"n": 5}, {}, {"kind": 2, "x": 1}, {"kind": 1, "n": 9}]
for d in items:
    match d:
        case {"kind": 1 | 3, "n": n}:
            print("kind 1/3", n)
        case {"kind": 2}:
            print("kind 2")
        case {"n": n} if n > 3:
            print("big n", n)
        case _:
            print("other", len(d))
for p in [P(0, 1), P(2, 2), P(5, 7)]:
    match p:
        case P(x=0 | 5, y=y):
            print("x in 0/5", y)
        case P(x=a, y=b) if a == b:
            print("diag", a)
        case _:
            print("else")
