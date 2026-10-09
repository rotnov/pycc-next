# #1264 (Part 3 of #1218): an annotated empty `list[int]`/`dict[str, int]`
# initialiser in `__init__` declares the slot from its written annotation.
# Every `__init__` call allocates a fresh container, so two instances never
# share one; the same annotated statement elsewhere is a plain store into
# the existing slot. Only scalars are printed: printing a container is a
# separate gap (#1018).

class Bag:
    def __init__(self, name: str) -> None:
        self.xs: list[int] = []
        self.d: dict[str, int] = {}
        self.name = name

    def add(self, v: int) -> None:
        self.xs.append(v)
        d = self.d
        d[self.name] = v

    def reset(self) -> None:
        self.xs: list[int] = []


class Sub(Bag):
    def total(self) -> int:
        t = 0
        ys = self.xs
        for x in ys:
            t = t + x
        return t


class Counter:
    def __init__(this) -> None:
        this.hits: dict[str, int] = {}

    def hit(this, key: str) -> int:
        n = this.hits.get(key, 0) + 1
        hits = this.hits
        hits[key] = n
        return n


def refill(b: Bag) -> None:
    b.xs: list[int] = []
    b.xs.append(7)


def main() -> None:
    a = Bag("a")
    b = Bag("b")
    a.add(3)
    print(len(a.xs), len(b.xs))
    print(a.d.get("a", -1), b.d.get("a", -1))
    s = Sub("s")
    s.add(4)
    s.add(5)
    print(s.total(), s.d["s"])
    s.reset()
    print(len(s.xs), s.total())
    refill(a)
    print(len(a.xs), a.xs[0], a.xs.pop(), len(a.xs))
    c = Counter()
    c.hit("x")
    c.hit("y")
    print(c.hit("x"), len(c.hits))


main()
