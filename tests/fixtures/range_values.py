def grid(w, fix=None):
    r = list(map(range, (len(w[0]), len(w[1]))))
    if fix is not None:
        r[fix] = range(len(w[fix]) - 1, len(w[fix]))
    out = []
    for i in r[0]:
        for j in r[1]:
            out.append(w[0][i] * 10 + w[1][j])
    return out, len(r)


def steps(n):
    down = range(n, 0, -2)
    total = 0
    for v in down:
        total += v
    again = [v * v for v in down]
    return total, again


def rows(k):
    spans = [range(k), range(1, k + 1)]
    built = [range(i, i + 2) for i in range(k)]
    acc = []
    for idx in range(len(built)):
        acc.append(sum([x for x in built[idx]]))
    for v in spans[1]:
        acc.append(v)
    return acc


def maybe(x=None):
    if x is not None:
        return x * 2
    return -1


w = [[1, 2, 3], [4, 5]]
print(grid(w), grid(w, 0), grid(w, 1))
print(steps(7), steps(0), rows(3), rows(0))
print(maybe(), maybe(21))
big = 10 ** 20
wide = [range(big, big + 2)]
for v in wide[0]:
    print(v)
try:
    print(grid([[1], []], 1))
except IndexError as e:
    print("IndexError", e)
