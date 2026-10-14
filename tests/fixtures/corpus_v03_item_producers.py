# An empty list or set item filled through a subscript: `graph[u].append(v)`.
n = 5
graph = {i: [] for i in range(n)}
for u, v in [(1, 2), (2, 3), (1, 4)]:
    graph[u].append(v)
    graph[v].append(u)
print(graph)
adj = [[] for _ in range(n)]
adj[0].append("x")
print(adj)
seen = {k: set() for k in "ab"}
seen["a"].add(3)
print(seen)
