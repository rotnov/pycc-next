# Item access on an instance of a class defining `__getitem__`,
# `__setitem__` and `__delitem__`: through `self`, a local and a global.
class DSU:
    def __init__(self, n):
        self.p = list(range(n))

    def _root(self, a):
        while self.p[a] != a:
            a = self.p[a]
        return a

    def __setitem__(self, a, b):
        r = self._root(b)
        while a != r:
            self.p[a], a = r, self.p[a]

    def __getitem__(self, a):
        self[a] = a
        return self.p[a]


class Grid(DSU):
    def __delitem__(self, a):
        self.p[a] = -1


def main():
    dsu = DSU(10)
    dsu[3] = 1
    dsu[5] = 3
    dsu[7] = 8
    print([dsu[i] for i in range(10)])
    print(dsu[5] == dsu[1], dsu[7] == dsu[1])


main()
g = Grid(4)
g[2] = 0
g[3] += 0
print(g[2], g[3], g.p)
del g[1]
print(g.p)
try:
    print(g[9])
except IndexError as e:
    print("IndexError", e)
