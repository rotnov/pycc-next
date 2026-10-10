"""Nested functions and lambdas capturing enclosing locals, `key=` and
`reverse=` on `sorted`/`min`/`max`/`list.sort`, comprehension tuple targets,
generator arguments, item deletion, set operators and empty containers whose
element type comes from later use."""


def counter() -> int:
    total = 0

    def add(k: int) -> None:
        nonlocal total
        total += k

    for i in range(5):
        add(i)
    add(10)
    return total


def fib_memo(n: int) -> int:
    memo: dict[int, int] = {}

    def go(k: int) -> int:
        if k < 2:
            return k
        if k in memo:
            return memo[k]
        r = go(k - 1) + go(k - 2)
        memo[k] = r
        return r

    return go(n)


def scaled(xs: list[int], factor: int) -> list[int]:
    def scale(x: int) -> int:
        return x * factor

    return [scale(x) for x in xs]


def dfs_count(n: int, edges: list[tuple[int, int]]) -> int:
    adj = [[] for _ in range(n)]
    for a, b in edges:
        adj[a].append(b)
        adj[b].append(a)
    seen = [False] * n

    def visit(u: int) -> None:
        seen[u] = True
        for v in adj[u]:
            if not seen[v]:
                visit(v)

    parts = 0
    for u in range(n):
        if not seen[u]:
            parts += 1
            visit(u)
    return parts


def keys_demo() -> None:
    words = ["pear", "fig", "banana", "kiwi", "apple"]
    print(sorted(words, key=len))
    print(sorted(words, key=lambda w: (len(w), w), reverse=True))
    print(sorted(words, reverse=True))
    print(min(words, key=len), max(words, key=len))
    pairs = [(3, "c"), (1, "a"), (2, "b"), (1, "z")]
    pairs.sort(key=lambda p: p[0])
    print(pairs)
    pairs.sort(key=lambda p: p[1], reverse=True)
    print(pairs)
    offset = 10
    print(sorted([5, 1, 4], key=lambda x: abs(x - offset)))
    d = {"a": 3, "b": 1, "c": 2}
    print(sorted(d.items(), key=lambda kv: kv[1]))
    print(max(d, key=lambda k: d[k]))


def comps_demo() -> None:
    pairs = [(1, "x"), (2, "y"), (3, "z")]
    print([n * 2 for n, _ in pairs], {c: n for n, c in pairs})
    print(sum(n for n, _ in pairs), max(n * n for n, _ in pairs))
    print(", ".join(c.upper() for _, c in pairs))
    print(any(n > 2 for n, _ in pairs), all(n > 2 for n, _ in pairs))


def del_and_sets() -> None:
    d = {1: "a", 2: "b", 3: "c"}
    del d[2]
    print(d)
    xs = [1, 2, 3, 4]
    del xs[1]
    del xs[-1]
    print(xs)
    a = {1, 2, 3}
    b = {3, 4}
    print(a | b, a & b, a - b, a ^ b, a <= a | b, b < a)
    alias = a
    a |= b
    print(alias, a)


def empties() -> None:
    seen = set()
    order = []
    counts = {}
    for w in "abracadabra":
        if w not in seen:
            seen.add(w)
            order.append(w)
        counts[w] = counts.get(w, 0) + 1
    print(order, counts, len(seen))
    grid = [[] for _ in range(3)]
    grid[1].append(5)
    print(grid)


print(counter(), fib_memo(30), scaled([1, 2, 3], 4))
print(dfs_count(6, [(0, 1), (1, 2), (3, 4)]))
keys_demo()
comps_demo()
del_and_sets()
empties()
