"""`int | float` lists and indices (v0.6 accept 3): a list of `int`s that
later stores a `float` (or the reverse) holds `int | float` elements, and an
`int | float` used as an index or a `range()` bound is checked to hold an
`int` at run time."""


def _distances(n: int):
    dist = [float("inf")] * n
    dist[0] = 0
    for v in range(1, n):
        if v % 3 != 0:
            dist[v] = dist[v - 1] + 1
    return dist


d = _distances(7)
print(d, sum(x for x in d if x != float("inf")))
for v in range(len(d)):
    if d[v] != float("inf"):
        print(v, d[v], [i for i in range(d[v])], "abcdefgh"[d[v]])

scores = [0 for _ in range(4)]
for i in range(4):
    scores[i] = i / 2
print(scores, max(scores))
counts = [0] * 3
counts[1] = 2.5
counts.append(4)
print(counts, sorted(counts))
half: int | float = 1.5
for bad in ["range", "list", "str", "slice"]:
    try:
        if bad == "range":
            print(list(range(half)))
        elif bad == "list":
            print([1, 2, 3][half])
        elif bad == "str":
            print("abc"[half])
        else:
            print([1, 2, 3][half:])
    except TypeError as e:
        print("TypeError:", e)
