# #1265 (Part 4 of #1218): an unannotated empty `[]` initialiser in
# `__init__` declares a list slot whose element type comes from an
# inherited slot or from the first `self.<attr>.append(v)` in the class's
# own methods. A reset in any method is typed from the slot. Only scalars
# are printed: printing a container is a separate gap (#1018).

class Log:
    def __init__(self) -> None:
        self.items = []
        self.ids = []
        self.ids.append(100)

    def add(self, v: int) -> None:
        self.items.append(v)

    def clear(self) -> None:
        self.items = []


class Tally(Log):
    def __init__(self) -> None:
        self.items = []
        self.ids = []

    def total(self) -> int:
        t = 0
        ys = self.items
        for y in ys:
            t = t + y
        return t


class Stack:
    def __init__(this) -> None:
        this.frames = []

    def push(this, f: int) -> None:
        this.frames.append(f)

    def top(this) -> int:
        return this.frames[len(this.frames) - 1]


def main() -> None:
    a = Log()
    b = Log()
    a.add(3)
    a.add(4)
    print(len(a.items), len(b.items), len(a.ids), a.ids[0])
    print(a.items[1], a.items.pop(), len(a.items))
    a.clear()
    print(len(a.items))
    t = Tally()
    t.add(5)
    t.add(6)
    print(t.total(), len(t.ids))
    s = Stack()
    s.push(15)
    s.push(25)
    print(s.top(), len(s.frames))


main()
