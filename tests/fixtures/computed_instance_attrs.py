class Fenwick:
    def __init__(self, n: int) -> None:
        self.n = n
        self.t = [0] * (n + 1)
        self.label = "fw" + str(n)
        self.half = n / 2
        self.big = n > 5

    def add(self, i: int, v: int) -> None:
        while i <= self.n:
            self.t[i] += v
            i += i & -i

    def query(self, i: int) -> int:
        s = 0
        while i > 0:
            s += self.t[i]
            i -= i & -i
        return s


class Graph:
    def __init__(self, n: int, edges: list[tuple[int, int]]) -> None:
        adj = [[] for _ in range(n)]
        for a, b in edges:
            adj[a].append(b)
            adj[b].append(a)
        self.adj = adj
        self.deg = [len(row) for row in adj]
        self.seen = {0: True}
        self.size = len(self.adj)
        self.root = Node(0, self.size)


class Node:
    def __init__(self, v: int, total: int) -> None:
        self.v = v
        self.share = v * 1.0 / total


f = Fenwick(10)
for i in range(1, 11):
    f.add(i, i * i)
print(f.query(5), f.query(10), f.label, f.half, f.big)
g = Graph(4, [(0, 1), (1, 2), (1, 3)])
print(g.adj, g.deg, g.seen, g.size, g.root.v, g.root.share)
print(Fenwick(3).t[7])
