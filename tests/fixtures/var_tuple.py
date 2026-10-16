# `tuple[T, ...]`: the homogeneous variadic tuple, compiled natively.
type Key = tuple[str, ...]


def parse_key(text: str) -> Key:
    parts = text.split(".")
    key: Key = (parts[0],)
    for part in parts[1:]:
        key += (part,)
    return key


def parents(key: Key) -> list[Key]:
    out: list[Key] = []
    for i in range(1, len(key)):
        out.append(key[:i])
    return out


def describe(key: Key) -> str:
    if not key:
        return "root"
    head, last = key[:-1], key[-1]
    return f"{last} under {head} ({len(key)} parts)"


def joined(header: Key, key: Key) -> Key:
    return header + key


def total(rows: tuple[list[int], ...]) -> int:
    n = 0
    for row in rows:
        for x in row:
            n += x
    return n


def nums(n: int) -> tuple[str, ...]:
    out: tuple[str, ...] = ()
    for i in range(n):
        out = out + (str(i * i),)
    return out


def rows(n: int) -> tuple[list[int], ...]:
    out: tuple[list[int], ...] = ()
    for i in range(n):
        out += ([i, i],)
    return out



def prefixes(empty: Key, a: Key) -> None:
    gen = (empty + a[:i] for i in range(1, len(a) + 1))
    for g in gen:
        print(g, end=" ")
    print()

empty: Key = ()
a = parse_key("a.b.c")
print(a, len(a), a[0], a[-1], a[1:], a[::-1], a[:0])
print(describe(a), describe(empty), describe(("x",)))
print(parents(a))
print(joined(("t",), a), joined(empty, empty), joined(a, ()))
print("b" in a, "z" in a, "z" not in a)
print(a == ("a", "b", "c"), a == parse_key("a.b.c"), a != a[:2], a < ("a", "c"), a[:2] < a)
print(a * 2, 2 * a[:1], a * 0)
print(hash(a) == hash(("a", "b", "c")))
seen: set[tuple[Key, int]] = set()
seen.add((a, 1))
seen.add((parse_key("a.b.c"), 1))
seen.add((a[:1], 0))
print(len(seen), (a, 1) in seen, (a[:2], 1) in seen)
for k, flag in sorted(seen):
    print(k, flag)
d: dict[Key, int] = {a: 3}
d[a[:1]] = 1
print(d[parse_key("a.b.c")], d[("a",)], len(d))
print([part.upper() for part in a], total(rows(5)), nums(4), str(nums(1)), repr(nums(0)))
prefixes(empty, a)
try:
    print(a[3])
except IndexError as e:
    print("IndexError:", e)
try:
    print(empty[-1])
except IndexError as e:
    print("IndexError:", e)
print(a[10:], a[-10:1], a[::2], a[1::-1])
try:
    print(a[::0])
except ValueError as e:
    print("ValueError:", e)
print(f"{a}", f"{empty}", str(("x",) + a[:1]))
r = rows(2)
alias = r[1:]
r[1].append(9)
print(r, alias)
