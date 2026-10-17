"""Big `int`s and `int | float` values beyond the inline range held in
containers: list, tuple, dict (keys and values) and set elements store
them as CPython does, and every operation on such a container reads them
back exactly."""

BIG = 10 ** 30
HUGE = 2 ** 100


def ints() -> None:
    xs = [1, 2, 3]
    xs.append(BIG)
    xs.append(-HUGE)
    xs[0] = HUGE + 1
    print(xs, len(xs))
    print(xs[3] + 1, xs[-1] * 2, xs[3] > xs[1])
    total = 0
    for x in xs:
        total += x
    print(total, sum(xs), min(xs), max(xs))
    ys = xs[1:4]
    zs = xs + ys
    print(ys, zs, xs * 2)
    print(sorted(xs), sorted(xs, reverse=True))
    print(BIG in xs, 10 ** 30 in xs, 7 in xs, xs.index(10 ** 30), xs.count(BIG))
    xs.sort()
    print(xs)
    xs.reverse()
    print(xs)
    print([x // 3 for x in xs if x > 2])
    a = xs.pop()
    b = xs.pop(0)
    print(a, b, xs)
    xs.remove(BIG)
    print(xs)
    xs.insert(1, 3 ** 70)
    xs[2] += BIG
    xs[0] *= 10 ** 20
    print(xs)
    del xs[1]
    print(xs, xs == [HUGE * 0 + xs[0], xs[1]], xs != [1])
    grid = [[0] * 3 for _ in range(2)]
    grid[1][2] = BIG
    grid[0][0] = grid[1][2] * grid[1][2]
    print(grid)
    first, second = [BIG, HUGE]
    print(first, second)
    xs.clear()
    print(xs)
    copy = list([BIG, 1])
    print(copy, list(reversed(copy)), [BIG] * 3)


def tuples() -> None:
    t = (BIG, 2, HUGE)
    print(t, t[0] + t[2], t == (10 ** 30, 2, 2 ** 100), t < (BIG, 3, 0))
    pairs = [(BIG, "a"), (1, "b"), (HUGE, "c")]
    pairs.sort()
    print(pairs)
    print(divmod(BIG, 7), divmod(BIG, 7) == (BIG // 7, BIG % 7))
    q, r = divmod(HUGE, 10 ** 9)
    print(q, r)


def dicts() -> None:
    d = {1: BIG, 2: 3}
    d[3] = HUGE
    d[2] = d[3] + d[1]
    print(d, d[1], d.get(3), d.get(9, BIG))
    keys = {BIG: "big", 5: "five"}
    keys[HUGE] = "huge"
    keys[-BIG] = "neg"
    print(keys, keys[10 ** 30], HUGE in keys, 3 in keys)
    print(sorted(keys), list(keys.values()))
    for k, v in d.items():
        print(k, v)
    print(d.pop(1), d.setdefault(7, BIG), d)
    counts: dict[int, int] = {}
    for x in [BIG, 1, BIG, HUGE, 1, BIG]:
        counts[x] = counts.get(x, 0) + 1
    print(counts)
    try:
        print(keys[BIG + 1])
    except KeyError as e:
        print("KeyError", e)


def sets() -> None:
    s = {1, BIG, 2}
    s.add(HUGE)
    s.add(10 ** 30)
    print(len(s), sorted(s), BIG in s, BIG + 1 in s)
    s.discard(BIG)
    print(sorted(s))
    u = s | {BIG, -HUGE}
    print(sorted(u), sorted(u & {HUGE, 3}), sorted(u - {1, 2}))
    print(sorted({x * BIG for x in range(3)}))


def nums(flag: bool) -> None:
    x = 1
    if flag:
        x = 1e300
    elif x > 0:
        x = 2.5e-300
    y = 0.5
    if flag:
        y = BIG
    xs = [x, y, 3, 1.5]
    xs.append(x * 10)
    print(xs, max(xs), min(xs), sorted(xs))
    d = {1: x}
    d[2] = y
    print(d, d[1] == x)
    s = {x, y, 1}
    print(len(s), x in s, y in s)
    t = (x, y)
    print(t, t == (x, y))
    for v in xs:
        print(v)
    keyed = {x: "x", y: "y"}
    print(keyed[x], keyed[y])


ints()
tuples()
dicts()
sets()
nums(True)
nums(False)
