# An operand read out of storage stays alive until its operation is done,
# even when a later operand runs code that drops the storage's reference.
# Each value below is built at run time (not a literal), so its only owner
# is the storage it is read from.


class P:
    items: list[str]

    def __init__(self, name: str) -> None:
        self.name = name
        self.items = [name + "!"]


class Box:
    def __init__(self, p: P) -> None:
        self.p = p


def fresh(tag: str, n: int) -> str:
    return tag + str(n)


def rename(p: P) -> str:
    p.name = fresh("renamed", 1)
    return fresh("a", 1)


def overwrite(xs: list[str]) -> str:
    xs[0] = fresh("other", 2)
    return "x"


def drop_items(p: P) -> str:
    p.items = [fresh("new", 0)]
    return fresh("item", 3)


def drop_index(p: P) -> int:
    p.items = [fresh("new", 0)]
    return 0


def swap(b: Box) -> int:
    b.p = P(fresh("q", 4))
    return 1


GL = [fresh("g", 6), fresh("h", 7)]
GB = Box(P(fresh("gp", 8)))


def compare_attribute() -> None:
    p = P(fresh("a", 1))
    print(p.name == rename(p), p.name)
    q = P(fresh("b", 2))
    print(q.name + rename(q), q.name)
    r = P(fresh("c", 3))
    print(r.name < rename(r) < "zzz")
    s = P(fresh("d", 4))
    print(fresh("x", 0) != s.name != rename(s))


def element_operands() -> None:
    xs = [fresh("e", 1), fresh("f", 2)]
    print(xs[0] + overwrite(xs), xs[0])
    ys = [fresh("e", 1)]
    print(ys[0] == overwrite(ys))
    p = P(fresh("w", 1))
    print(p.items[drop_index(p)], p.items)


def receivers() -> None:
    p = P(fresh("r", 1))
    p.items.append(drop_items(p))
    print(p.items)
    b = Box(P(fresh("s", 2)))
    print(len(b.p.name) + swap(b), b.p.name)
    b2 = Box(P(fresh("t", 3)))
    b2.p.name = fresh("u", 4) + str(swap(b2))
    print(b2.p.name)
    b3 = Box(P(fresh("y", 5)))
    b3.p.items[0] = fresh("v", 6) + str(swap(b3))
    print(b3.p.items)


def globals_as_operands() -> None:
    print(GL[0] + overwrite(GL), GL)
    print(GB.p.name + str(swap(GB)), GB.p.name)
    print(GB.p.items[0] + drop_items(GB.p), GB.p.items)


def print_arguments() -> None:
    p = P(fresh("v", 1))
    print(p.items, drop_items(p), p.items)
    q = P(fresh("m", 2))
    print(q.name, rename(q), q.name)
    b = Box(P(fresh("n", 3)))
    print(b.p.items, swap(b), b.p.items)
    c = Box(P(fresh("o", 4)))
    print(f"{c.p.items}{swap(c)}{c.p.name}")


def fresh_list() -> list[str]:
    return [fresh("l", 1)]


def failing_reads() -> None:
    # A read that raises yields no value to take a reference to.
    try:
        print(fresh_list()[5] + fresh("x", 1))
    except IndexError as e:
        print("IndexError", e)
    p = P(fresh("z", 3))
    try:
        print(p.items[7] == rename(p))
    except IndexError as e:
        print("IndexError", e, p.name)


compare_attribute()
failing_reads()
element_operands()
receivers()
globals_as_operands()
print_arguments()
