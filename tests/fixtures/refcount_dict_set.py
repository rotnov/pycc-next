# Dicts and sets are reference counted: aliases share one object, a
# rebinding frees the old one only when nothing else holds it, and a loop
# keeps iterating the object it started with.


class Node:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        return self.v % 3

    def __eq__(self, other: "Node") -> bool:
        return self.v == other.v


class Index:
    counts: dict[str, int]

    def __init__(self) -> None:
        self.counts = {}

    def bump(self, key: str) -> int:
        self.counts[key] = self.counts.get(key, 0) + 1
        return self.counts[key]


def key(i: int) -> str:
    return "k" + str(i)


def make(n: int) -> dict[str, int]:
    d: dict[str, int] = {}
    for i in range(n):
        d[key(i)] = i * i
    return d


def total(d: dict[str, int]) -> int:
    t = 0
    for k in d:
        t += d[k]
    return t


def aliasing() -> None:
    a = make(3)
    b = a
    a = make(1)
    print(total(a), total(b), len(b))
    b[key(9)] = 9
    print(sorted_keys(b))
    for k in b:
        b = {key(0): 0}
        print(k)
    print(len(b))


def sorted_keys(d: dict[str, int]) -> str:
    out = ""
    for k in d:
        out += k + ","
    return out


def sets() -> None:
    s = {1, 2, 3}
    t = s
    s = {4}
    print(len(s), len(t))
    for x in t:
        t = {9}
        print(x)
    f = frozenset(t)
    t = {7}
    print(len(f))
    nodes = {Node(1), Node(4), Node(1), Node(7)}
    print(len(nodes))
    nodes.add(Node(4))
    nodes.add(Node(10))
    print(len(nodes))
    copy = frozenset(nodes)
    nodes = {Node(0)}
    print(len(copy))
    squares = {i * i for i in range(5)}
    by_key = {key(i): i for i in range(4)}
    print(len(squares), total(by_key))


def attributes() -> None:
    ix = Index()
    for word in ["ab", "c", "ab", "def"]:
        ix.bump(word)
    print(total(ix.counts), len(ix.counts))
    held = ix.counts
    ix = Index()
    print(total(held), len(ix.counts))
    d = make(4)
    print(d.get(key(3), -1), make(2)[key(1)], len(make(5)))


aliasing()
sets()
attributes()
