# Found by the differential fuzzer (fuzz/): `for a, b in zip(xs, ys)` takes
# each sequence's length as it stands at every step, as CPython's iterators
# do: a body that shrinks, grows or rebinds a sequence used to index past its
# end (IndexError) or stop at the starting length.


def _shrink(xs: list[int]) -> float:
    for a, b in zip(xs, "MiXeD"):
        print(a, b)
        del xs[0]
    return 1.0


print(_shrink([6, 3, 5]))
zs: list[int] = [1, 2]
for a, b in zip(zs, zs):
    zs.append(a + b)
    if len(zs) > 6:
        break
print(zs)
ws: list[str] = ["p", "q", "r"]
for i, w in zip([1, 2, 3], ws):
    ws = ["x"]
    print(i, w)
else:
    print("else", ws)
n: list[int] = []
for a, b in zip(n, [1]):
    print("never")
else:
    print("empty else")
for a, b, c in zip([1, 2, 3, 4], "ab", [9.5, 8.5]):
    if a == 1:
        continue
    print(a, b, c)
