# Found by the differential fuzzer (fuzz/): where a nan lands in a sorted
# list of floats depends on exactly which `<` comparisons the sort makes (a
# nan compares false both ways), so sorting must be CPython's own timsort:
# its run detection, binary insertion, powersort merges and galloping.
nan = float('nan')
print(sorted([2.675, 0, nan]))
print(26.0, sorted([nan, 758.92, 42.4]), sep=' ')
print(sorted([3.0, nan, 1.0, 2.0]), sorted([3.0, 2.0, nan, 1.0, 0.5]))
print(sorted([nan, 1.0, 0.5], reverse=True), sorted([2.0, nan, 1.0], key=lambda x: -x))
xs: list[float] = [5.0, 4.0, nan, 4.0, 3.0, 9.0, 1.0, nan, 0.0]
xs.sort()
print(xs)


def _gen(seed: int, n: int) -> list[float]:
    out: list[float] = []
    s = seed
    v = 0.0
    for i in range(n):
        s = (s * 1103515245 + 12345) % 2147483648
        r = s % 1000
        if r < 15:
            out.append(nan)
        elif r < 450:
            v = v + float(r % 7)
            out.append(v)
        elif r < 650:
            v = v - float(r % 5)
            out.append(v)
        else:
            out.append(float(r % 50))
    return out


# Long enough for several runs, merges in both directions and galloping.
for seed in [1, 7, 12]:
    ys = _gen(seed, 400)
    zs = sorted(ys)
    print(seed, [k for k in range(len(zs)) if zs[k] != zs[k]], zs[::37])
