class Grower:
    """`__lt__` appends to the list being sorted (it reads as empty)."""

    def __init__(self, n: int, sink: list["Grower"]) -> None:
        self.n = n
        self.sink = sink

    def __lt__(self, other: "Grower") -> bool:
        self.sink.append(Grower(-1, self.sink))
        return self.n < other.n

    def __repr__(self) -> str:
        return f"G{self.n}"


class Clearer:
    """`__lt__` clears the list being sorted, and reports its length."""

    def __init__(self, n: int, seen: list[int], sink: list["Clearer"]) -> None:
        self.n = n
        self.seen = seen
        self.sink = sink

    def __lt__(self, other: "Clearer") -> bool:
        self.seen.append(len(self.sink))
        self.sink.clear()
        return self.n < other.n

    def __repr__(self) -> str:
        return f"C{self.n}"


class Boom:
    """`__lt__` raises on the first comparison."""

    def __init__(self, n: int) -> None:
        self.n = n

    def __lt__(self, other: "Boom") -> bool:
        raise RuntimeError(f"cannot order {self.n} and {other.n}")

    def __repr__(self) -> str:
        return f"B{self.n}"


class Item:
    def __init__(self, n: int) -> None:
        self.n = n

    def __lt__(self, other: "Item") -> bool:
        return self.n < other.n

    def __repr__(self) -> str:
        return f"I{self.n}"


held: list[Item] = [Item(3), Item(1), Item(2)]


def rebind(item: Item) -> Item:
    # Rebinds the only global holding the list being sorted.
    global held
    held = []
    return item


def main() -> None:
    gs: list[Grower] = []
    gs.extend([Grower(3, gs), Grower(1, gs), Grower(2, gs)])
    try:
        gs.sort()
    except ValueError as e:
        print("ValueError:", e)
    print(gs)

    seen: list[int] = []
    cs: list[Clearer] = []
    cs.extend([Clearer(2, seen, cs), Clearer(1, seen, cs)])
    try:
        cs.sort()
    except ValueError as e:
        print("ValueError:", e)
    print(cs, seen)

    bs = [Boom(2), Boom(1), Boom(3)]
    try:
        bs.sort()
    except RuntimeError as e:
        print("RuntimeError:", e)
    print(bs)
    try:
        print(sorted(bs))
    except RuntimeError as e:
        print("RuntimeError:", e)
    print(bs)

    held.sort(key=rebind)
    print(held)


main()
