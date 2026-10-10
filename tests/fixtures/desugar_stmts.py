# Statement-level desugarings (pycc_ast::desugar): loop targets and
# iterables, computed subscript stores and augmented assignments.
g = [[0] * 3 for _ in range(3)]
for i in range(3):
    for j in range(3):
        g[i][j] = i * 3 + j
g[2][0] += 100
print(g)
a = [5, 1, 4]
a[0], a[2] = a[2], a[0]
print(a)
idx = 0
a[idx + 1] += 10
a[len(a) - 1] -= 1
print(a)
for i, v in enumerate(a):
    if i == 1:
        continue
    print(i, v)
for i, v in enumerate(a, 1):
    print(i, v)
for x, y in zip(a, [7, 8]):
    print(x, y)
for x in a[1:]:
    print(x)
for x in sorted(a):
    print(x)
for x in [3, 2, 1]:
    print(x)
d = {"a": 1, "b": 2}
for k, v in d.items():
    print(k, v)
total = 0
for i in range(2):
    for j, w in enumerate([10, 20]):
        total += i * j + w
print(total)
p, q = 1, 2
p, q = q, p
print(p, q)
u, v, w = 1, 2, 3
m, n = [4, 5]
print(u, v, w, m, n)
try:
    m, n = [1, 2, 3]
except ValueError as e:
    print(e)
calls = [0]
def key() -> int:
    calls[0] += 1
    return 1
a[key()] += 5
a[key() - 1] *= 2
print(a, calls)
print(a[5])
