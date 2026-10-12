# Unannotated public functions and methods of a native program: each
# signature is inferred from its call sites, in strict mode too.


def gcd(a, b):
    while b:
        a, b = b, a % b
    return a


def scale(xs, k):
    return [x * k for x in xs]


def fib(n):
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


def describe(name, score=0):
    return name + ": " + str(score)


def solve():
    n = 6
    total = 0
    for i in reversed(range(n)):
        total = total * 2 + i
    print(total, list(reversed(range(1, 10, 3))), list(map(str, range(3))))


class UnionFind:
    def __init__(self, n, label):
        self.n = n
        self.label = label
        self.parent = list(range(n))
        self.size = [1] * (n + 0)

    def find(self, x):
        while self.parent[x] != x:
            self.parent[x] = self.parent[self.parent[x]]
            x = self.parent[x]
        return x

    def union(self, a, b):
        a = self.find(a)
        b = self.find(b)
        if a == b:
            return False
        if self.size[a] < self.size[b]:
            a, b = b, a
        self.parent[b] = a
        self.size[a] += self.size[b]
        return True

    def count(self, x):
        return self.size[self.find(x)]


print(gcd(84, 36), gcd(7, 0), gcd(-12, 18))
print(scale([1, 2, 3], 4), scale([], 2))
print(fib(15))
print(describe("ann"), describe("bob", 7))
solve()
uf = UnionFind(8, "uf")
print(uf.n, uf.label)
print(uf.union(0, 1), uf.union(1, 2), uf.union(2, 0), uf.union(5, 6))
print(uf.count(0), uf.count(5), uf.count(7), uf.find(2) == uf.find(0))
print(gcd(10, 0) // gcd(0, 0))
