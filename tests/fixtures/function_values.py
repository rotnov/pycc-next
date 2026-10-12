from typing import Callable, Iterator


def add(a: int, b: int) -> int:
    return a + b


def mul(a: int, b: int) -> int:
    return a * b


def scale(k: int) -> Callable[[int], int]:
    def by(x: int) -> int:
        return x * k

    return by


def count(n: int) -> Iterator[int]:
    i = 0
    while i < n:
        yield i
        i += 1


class Holder:
    def __init__(self, f: Callable[[int, int], int]) -> None:
        self.f = f

    def apply(self, a: int, b: int) -> int:
        return self.f(a, b)


class Registry:
    handlers: dict[str, Callable[[int], int]]

    def __init__(self) -> None:
        self.handlers = {}

    def register(self, name: str, f: Callable[[int], int]) -> None:
        self.handlers[name] = f

    def run(self, name: str, x: int) -> int:
        return self.handlers[name](x)


OPS: list[Callable[[int, int], int]] = [add, mul]


def rebinding() -> None:
    # A nested function assigning `nonlocal` variables (a plain assignment,
    # not only `+=`), one of them a function value called afterwards.
    k = 10
    g = double

    def rb() -> int:
        nonlocal g, k
        g = ident
        k = k + 3
        return 5

    print(g(rb()), k, g(7))
    cell = [double]
    h = cell[0]
    print(h(7))
    for f in cell:
        print(f(1))


def ident(x: int) -> int:
    return x


def double(x: int) -> int:
    return 2 * x


def main() -> None:
    for op in OPS:
        print(op(3, 4))
    print(OPS[1](5, 6))
    table: dict[str, Callable[[int, int], int]] = {"add": add, "mul": mul}
    print(table["add"](1, 2), table["mul"](3, 3))
    fs = [scale(i) for i in range(1, 4)]
    fs.append(scale(10))
    print([f(2) for f in fs])
    last = fs.pop()
    print(last(4), len(fs))
    h = Holder(mul)
    print(h.f(7, 8), h.apply(2, 3))
    h.f = add
    print(h.f(7, 8), h.apply(2, 3))
    r = Registry()
    r.register("double", scale(2))
    r.register("triple", scale(3))
    r.register("double", scale(4))
    print(r.run("double", 5), r.run("triple", 5), len(r.handlers))

    # Printing a function or a generator names it as CPython does; the
    # test compares the address as `0x...`.
    print(add)
    print(scale(1))
    print(f"[{mul}]")
    print(str(fs[0]))
    print(fs)
    print(table)
    g = count(2)
    print(g)
    print(str(g).startswith("<generator object count at 0x"))
    print(list(g))


main()
rebinding()
