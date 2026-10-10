"""Generic dicts, sets and tuples: every key, value and element type the
runtime objects store, CPython's set iteration order, and the container
builtins and methods over them."""


def dicts() -> None:
    d = {1: "a", 2: "b"}
    d[3] = "c"
    d[1] = "z"
    print(d, len(d), d[2], 3 in d, 7 in d, 7 not in d)
    for k in d:
        print(k, d[k])
    counts: dict[str, int] = {}
    for word in "the cat and the hat and the bat".split():
        counts[word] = counts.get(word, 0) + 1
    print(counts)
    grid = {(0, 0): 1.5, (1, 2): -2.0}
    grid[(3, 4)] = 0.25
    print(grid, grid[(1, 2)], (0, 0) in grid, (9, 9) in grid)
    adj: dict[int, list[int]] = {}
    for a, b in [(1, 2), (1, 3), (2, 3)]:
        if a not in adj:
            adj[a] = []
        adj[a].append(b)
    print(adj, adj[1], len(adj[2]))
    print(sorted(d), list(d), list(d.keys()), list(d.values()), list(d.items()))
    print(d.get(9, "none"), d.get(2, "none"), d.pop(2), d.pop(9, "gone"))
    print(d)
    print(d.setdefault(5, "e"), d.setdefault(1, "unused"))
    print(d)
    e = dict(d)
    e[100] = "x"
    print(d == e, len(e), {1: [1, 2]} == {1: [1, 2]}, {1: 2} != {1: 3})
    squares = {i: i * i for i in range(6) if i % 2 == 0}
    print(squares, dict([(1, 2), (3, 4)]))
    del squares[2]
    print(squares.popitem())
    print(squares)
    nested = {"a": {"x": 1}, "b": {}}
    nested["b"]["y"] = 2
    print(nested)


def sets() -> None:
    s = {3, 1, 2, 100, 33}
    s.add(7)
    print(s, len(s), 2 in s, 99 in s)
    for x in s:
        print(x)
    seen = set()
    for v in [5, 3, 5, 9, 3, 1, 64, 1000, -1]:
        seen.add(v)
    print(seen, sorted(seen), min(seen), max(seen))
    words = set("mississippi")
    print(len(words), sorted(words))
    a = {i * 37 % 101 for i in range(40)}
    b = set(range(10, 30))
    print(a | b)
    print(a & b, a - b, b - a)
    print(a ^ b)
    print(a <= a | b, (a & b) <= a, {1, 2}.isdisjoint({3}))
    t = {(1, 2), (0, 5), (1, 2)}
    print(t, (0, 5) in t)
    f = {0.5, 2.25, -1.0, 3.0}
    print(f)
    s.discard(100)
    s.remove(1)
    print(s)
    print(s.pop())
    print(s)
    c = s.copy()
    c.update([40, 41])
    print(s, c, set([2, 1, 2]))
    print({"x"} == {"x"}, s != c)


def tuples() -> None:
    t = ("a", 1, [1, 2])
    print(t, t[0], t[1], t[2], len(t[2]))
    pairs = [(3, "c"), (1, "a"), (2, "b"), (1, "Z")]
    print(sorted(pairs), min(pairs), max(pairs))
    pairs.sort()
    print(pairs, pairs[0][1])
    for i, x in enumerate(["p", "q", "r"]):
        print(i, x)
    print(list(enumerate([5, 6], 1)), list(zip([1, 2, 3], ["a", "b"])))
    print([(n, c) for n, c in zip([1, 2], "xy")])
    rows = [(1, 2, 3), (4, 5, 6)]
    print([a + b + c for a, b, c in rows], {k: v for k, v in [(1, "one")]})
    print((1, "a") < (1, "b"), (2, 1) > (1, 9), (1, 2) == (1, 2), ("x",))
    nested = ((1, 2), ("k", [3]))
    print(nested, nested[1][1])


dicts()
sets()
tuples()
missing: dict[str, int] = {"a": 1}
print(missing["b"])
