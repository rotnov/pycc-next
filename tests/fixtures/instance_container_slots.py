# #1262 (Part 1 of #1218): `list[int]` and `dict[str, int]` instance
# attributes seeded from an `__init__` parameter. The slot holds the
# container itself, so a mutation through an alias is visible through the
# attribute, as in CPython. Only scalars are printed: printing a container
# is a separate gap (#1018).

class Bag:
    def __init__(self, xs: list[int], counts: dict[str, int]) -> None:
        self.xs = xs
        self.counts = counts
        self.label = "bag"

    def total(self) -> int:
        items = self.xs
        acc = 0
        for x in items:
            acc = acc + x
        return acc

    def count_of(self, key: str) -> int:
        return self.counts[key]

    def replace(self, other: list[int]) -> None:
        self.xs = other


class Tagged(Bag):
    def first(self) -> int:
        return self.xs[0]


def main() -> None:
    b = Bag([1, 2, 3], {"a": 1, "b": 2})
    print(len(b.xs))
    print(b.xs[0], b.xs[2])
    print(b.counts["b"])
    print(b.total())
    ys = b.xs
    ys.append(4)
    print(len(b.xs), b.xs[3])
    print(b.total())
    d = b.counts
    d["c"] = 7
    print(len(b.counts), b.count_of("c"))
    b.replace([10, 20])
    print(len(b.xs), b.total())
    print(len(ys))
    t = Tagged([5, 6], {"z": 26})
    print(t.first(), t.total(), t.count_of("z"), t.label)


main()
