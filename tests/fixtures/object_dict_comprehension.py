import sys

o = sys.argv[:1] + ["a", "b"]
n = len(o) + 1
by_index = {i: o[1] for i in range(n)}
print(len(by_index), by_index)
graph = {i: [] for i in range(n)}
graph[0].append(5)
graph[1].append("x")
print(graph, graph[2] == [])
renamed = {str(k): v for k, v in [(1, o[1:]), (2, o[2:])] if k}
print(len(renamed), sorted(renamed), renamed)
odd = {o[1]: i for i in range(4) if i % 2}
print(odd == {o[1]: 3}, odd)
lengths = {x: len(x) for x in o[1:]}
print(lengths)
shouted = {x: x + "!" for x in o[1:] if x != "b"}
print(shouted)
try:
    graph[sys.argv]
except TypeError as exc:
    print("TypeError", exc)
