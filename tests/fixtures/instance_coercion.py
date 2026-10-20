"""A CPython object passed by compiled code to a parameter annotated with a class
compiled in the module (`tests/instance_coercion.rs`)."""


class Conf:
    def __init__(self, n: int) -> None:
        self.n = n

    def scaled(self, k: int) -> int:
        return self.n * k


class Holder:
    def __init__(self, c, label) -> None:
        self.c = c
        self.label = label


def use(c: Conf, k: int) -> int:
    return c.scaled(k) + c.n


def via(h) -> int:
    return use(h.c, 3)


def main() -> None:
    holders = [Holder(Conf(n), str(n)) for n in range(4)]
    print([via(h) for h in holders])
    first = holders[0].c
    print(use(first, 10), first is holders[0].c)


if __name__ == "__main__":
    main()
