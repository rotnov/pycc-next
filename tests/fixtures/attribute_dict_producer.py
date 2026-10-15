# An unannotated `self.d = {}` is typed from the first `self.d[k] = v` in the
# class's own methods, and an empty attribute container whose producer reads
# an unannotated method parameter is typed once the parameter is.


class Grid:
    def __init__(self, n):
        self.n = n
        self.filled = {}
        self.counts = {}
        self.log = []

    def write(self, x, y, t):
        if (x, y) in self.counts:
            self.counts[(x, y)] += 1
        else:
            self.counts[(x, y)] = 1
        self.filled[(x, y)] = t
        self.log.append((x, y))

    def clear(self, x, y):
        if (x, y) not in self.filled:
            return -1
        return self.filled.pop((x, y))


class Names:
    def __init__(self) -> None:
        self.index = {}

    def add(self, name: str) -> int:
        if name not in self.index:
            self.index[name] = len(self.index)
        return self.index[name]


g = Grid(3)
g.write(1, 2, 0)
g.write(2, 2, 1)
g.write(1, 2, 1)
print(g.filled, g.counts, g.log)
print(g.clear(1, 2), g.clear(1, 2), len(g.filled))
names = Names()
print([names.add(w) for w in "a b a c b".split()], names.index)
