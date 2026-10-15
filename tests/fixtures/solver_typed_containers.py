# Empty containers typed from producers that read unannotated functions,
# `set.update`/`list.extend` of any iterable, and a tuple display passed to
# `sorted`/`min`/`max`/`sum`.


class DSU:
    def __init__(self, n):
        self.p = list(range(n))

    def find(self, x):
        if self.p[x] == x:
            return x
        self.p[x] = self.find(self.p[x])
        return self.p[x]

    def union(self, a, b):
        self.p[self.find(a)] = self.find(b)


dsu = DSU(6)


def roots(n):
    seen = set()
    for i in range(n):
        seen.add(dsu.find(i))
    return sorted(seen)


def pairs(groups):
    out = set()
    for group in groups:
        out.update([(group[0], x) for x in group[1:]])
    flat = []
    for group in groups:
        flat.extend(group)
    return sorted(out), flat


class Counts:
    def __init__(self):
        self.by_key = {}
        self.order = []

    def add(self, key, value):
        if key not in self.by_key:
            self.order.append(key)
            self.by_key[key] = 0
        self.by_key[key] += value


for a, b in [(0, 1), (2, 3), (1, 3)]:
    dsu.union(a, b)
print(roots(6))
print(pairs([[1, 2, 3], [2, 4]]))
c = Counts()
c.add("a", 2)
c.add("b", 1)
c.add("a", 5)
print(c.by_key, c.order)
p, q = 7, 3
p, q = sorted((p, q))
print(p, q, min((4, 2, 9)), max((1.5, 0.5)), sum((1, 2, 3)), sorted(("b", "a")))
