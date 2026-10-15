from dataclasses import dataclass


class V:
    def __init__(self, n: int) -> None:
        self.n = n

    def __repr__(self) -> str:
        return f"V({self.n})"


@dataclass(eq=False)
class P:
    x: int
    vs: list[V]


xs = [V(1), V(2)]
print(xs)
print(f"{xs} and {xs!r}", str(xs), repr(xs))
print([[V(3)], []])
empty: list[V] = []
print(empty, P(1, xs))


def pair(n: int) -> tuple[V, str]:
    return (V(n), "v" + str(n))


def main() -> None:
    ys = [V(5)]
    print(ys, len(str(ys)))
    # Tuples holding instances, alone and in a list.
    t = (V(1), "a", 2.5)
    print(t, [t, t], (V(9),), repr(pair(3)), f"{pair(4)}")
    print([pair(i) for i in range(3)])


main()
