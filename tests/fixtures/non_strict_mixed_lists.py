# Non-strict mode: a list that comes to hold values of two types is
# CPython's own list, as every Python list is.
out = []
for t in range(3):
    bob = float("inf")
    if t > 0:
        bob = t
    out.append(0)
    out.append(bob)
for e in out:
    print(e)

n = 4
dist = [float("inf")] * n
dist[1] = 0
dist[2] = dist[1] + 1.5
print(dist)


def f(n):
    res = [0] * n
    res[0] = "x"
    return res


print(f(3))
