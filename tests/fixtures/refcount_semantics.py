# Reference-counted lists and instances: aliasing, rebinding and release
# must never free an object something still names, and never change what a
# program prints.


class Cell:
    def __init__(self, v: int, label: str) -> None:
        self.v = v
        self.label = label


class Pair:
    def __init__(self, a: Cell, b: Cell) -> None:
        self.a = a
        self.b = b


class Chain:
    def __init__(self, v: int, nxt: Cell) -> None:
        self.v = v
        self.nxt = nxt


class Box:
    items: list[Cell]
    rows: list[list[int]]

    def __init__(self) -> None:
        self.items = []
        self.rows = []

    def push(self, c: Cell) -> "Box":
        self.items.append(c)
        return self

    def swap(self) -> int:
        self.items = [Cell(-1, "fresh")]
        return 7


def make(i: int) -> Cell:
    return Cell(i, f"c{i}")


def first(xs: list[Cell]) -> Cell:
    return xs[0]


def keep(c: Cell, into: list[Cell]) -> None:
    into.append(c)


def replace_shared() -> int:
    shared.pop()
    shared.append(Cell(100, "new"))
    return 1


shared: list[Cell] = [Cell(1, "old")]


def main() -> None:
    # An alias outlives the name it was read from.
    a = make(1)
    b = a
    a = make(2)
    print(a.v, b.v, b.label)
    # A list element outlives the list.
    xs = [make(3), make(4)]
    c = xs[1]
    xs = [make(5)]
    print(c.v, c.label, xs[0].v)
    # A pair shares one cell twice.
    p = Pair(c, c)
    c = make(6)
    p.a = make(7)
    print(p.a.v, p.b.v, p.b.label, c.v)
    # An attribute of a fresh object.
    print(make(8).label, Pair(make(9), make(10)).b.v)
    # Elements kept by another list survive the first.
    kept: list[Cell] = []
    for i in range(5):
        keep(make(i * 10), kept)
    total = 0
    for k in kept:
        total += k.v
    print(total, len(kept), first(kept).label)
    # Nested lists share their inner list.
    row = [1, 2]
    grid = [row, row, [3]]
    row = [9]
    grid[1][0] = 50
    print(grid[0][0], grid[1][0], grid[2][0], row[0])
    # A receiver rebound by its own argument (CPython keeps it alive).
    box = Box()
    box.push(make(11)).push(make(12))
    old = box.items
    box.items.append(Cell(box.swap(), "arg"))
    print(len(box.items), box.items[0].label, len(old), old[2].v)
    # A loop over a list its body rebinds.
    ys = [make(20), make(21), make(22)]
    for y in ys:
        ys = [make(0)]
        total += y.v
    print(total, len(ys))
    # A loop over a module global whose element a call removes.
    for s in shared:
        replace_shared()
        print(s.v, s.label)
    print(shared[0].label)
    # Each rebinding releases the previous chain and its cell.
    head = Cell(0, "head")
    chain = Chain(0, head)
    for i in range(200000):
        chain = Chain(i, Cell(i, "x"))
    print(chain.v, chain.nxt.v, head.label)
    # Copies of lists of instances keep their elements.
    zs = [make(30), make(31)]
    copy = zs[:]
    doubled = zs * 2
    zs = []
    print(copy[1].v, doubled[3].label, len(zs))
    popped = copy.pop()
    copy = []
    print(popped.v, popped.label)


main()
