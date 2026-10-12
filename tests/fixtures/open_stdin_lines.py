# Iterating `open(0)` yields standard input's lines, newlines kept.
_, *t = open(0)
for p in t:
    x, k = p.split()
    print(x[::-1], int(k) * 2, p.endswith("\n"), len(p))
