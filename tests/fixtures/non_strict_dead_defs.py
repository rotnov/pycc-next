# Non-strict mode: unannotated definitions nothing calls are not compiled,
# so they cannot fall back to CPython either.
import sys


def list2d(a, b, c):
    return [[c] * b for i in range(a)]


def YES():
    print("YES")


unused = lambda x: [x, None]


class Tree:
    def __init__(self, n: int):
        self.data = [0] * (n + 1)

    def add(self, i, v):
        while i < len(self.data):
            self.data[i] += v
            i += i & -i

    def total(self, i):
        s = 0
        while i > 0:
            s += self.data[i]
            i -= i & -i
        return s

    def dump(self, sep):
        return sep.join(map(str, self.data))


def main():
    t = Tree(8)
    for i, v in enumerate([5, 3, 7, 1], 1):
        t.add(i, v)
    print(t.total(2), t.total(4), list(reversed(range(4))))


main()
sys.stdout.write("done\n")
