# Iterating a tuple whose elements share one type: a `for` loop, a
# comprehension over the name, and `map`/`join` over it.
ret = []
for i in range(3):
    ret.append((i, i * i, i + 10))
for el in ret:
    print(" ".join(map(str, el)))
t = ("a", "bc", "def")
for s in t:
    print(s, len(s))
print([len(s) for s in t], sum(x for x in ret[2]))
w = (1.5, 2.5)
total = 0.0
for x in w:
    total += x
print(total)
