# #1263 (Part 2 of #1218): `.append()`, `.pop()` and `.get(k, default)` on
# an instance-attribute receiver (`self.xs.append(v)`). The receiver is the
# container the slot holds, so a mutation through it is visible through
# every alias, as in CPython, and the receiver is evaluated before the
# call's arguments. Only scalars are printed: printing a container is a
# separate gap (#1018).

class Stack:
    def __init__(self, xs: list[int], counts: dict[str, int]) -> None:
        self.xs = xs
        self.counts = counts

    def push(self, v: int) -> None:
        self.xs.append(v)

    def take(self) -> int:
        return self.xs.pop()

    def look(self, key: str) -> int:
        return self.counts.get(key, -1)

    def swap(self) -> int:
        self.xs = [100]
        return 7

    def swap_counts(self) -> int:
        self.counts = {"k": 50}
        return 0

    def drain(self) -> int:
        acc = 0
        while len(self.xs) > 0:
            acc = acc + self.xs.pop()
        return acc


class Holder:
    def __init__(self, xs: list[int]) -> None:
        self._xs = xs

    @property
    def items(self) -> list[int]:
        print("items")
        return self._xs

    # An instance-typed slot is not supported yet, so the chain's middle
    # link is a property returning the module-global `Stack`.
    @property
    def inner(self) -> Stack:
        return g

    def noisy(self) -> int:
        print("noisy")
        return 5


def mutate_global() -> None:
    g.xs.append(g.counts.get("z", 40))


def main() -> None:
    s = Stack([1, 2], {"a": 1})
    s.push(3)
    print(len(s.xs), s.xs[2])
    print(s.take(), len(s.xs))
    print(s.look("a"), s.look("missing"))
    ys = s.xs
    s.xs.append(9)
    print(len(ys), ys[2])
    ys.append(10)
    print(s.xs.pop(), s.xs.pop())
    print(s.drain(), len(s.xs))
    s.xs.append(8)
    print(s.xs.pop())
    # Evaluation order: the receiver is read before `swap()` rebinds it.
    old = s.xs
    s.xs.append(s.swap())
    print(len(old), old[0], len(s.xs), s.xs[0])
    s.xs.append(s.swap())
    print(len(s.xs), s.xs[0])
    print(s.counts.get("k", s.swap_counts()), s.counts.get("k", 0))
    # A property receiver runs its getter before the argument.
    h = Holder([1])
    h.items.append(h.noisy())
    print(len(h._xs), h._xs[1])
    # A property chain ending in a slot.
    h.inner.xs.append(11)
    print(h.inner.xs.pop(), len(g.xs))
    popped = [s.xs.pop() for _ in range(1)]
    print(len(popped), popped[0])


g = Stack([0], {"y": 2})
g.xs.append(1)
mutate_global()
mutate_global()
print(len(g.xs), g.xs[1], g.xs.pop(), g.counts.get("y", 0))
main()
# An empty attribute receiver raises a catchable `IndexError`.
e = Stack([5], {"a": 1})
e.xs.pop()
try:
    e.xs.pop()
except IndexError as err:
    print(err)
