xs = [1, 2, 3, 4, 5]
ys = xs[1:4]
for v in ys:
    print(v)
print(len(ys))

xs.append(6)
last = xs.pop()
print(last)
print(len(xs))

named = {f"n{i}": i * i for i in range(4)}
# 999, not -1: unary minus (`Expr::UnaryOp`/`USub`) has no `pycc_hir`
# lowering arm at all, so any negative literal is rejected with C0001
# today -- a broader, project-wide gap than D-108/D-118's own narrower
# "no negative index/slice-bound" rule. 999 is an equally unambiguous
# missing-key sentinel here since every real value is in range [0, 9].
print(named.get("n2", 999))
print(named.get("missing", 999))

evens = {x for x in range(10) if x % 2 == 0}
evens.add(100)
print(len(evens))
