"""`int | float` inferred where one name is bound to both (v0.6 accept 3):
a `float('inf')` sentinel an `int` replaces, an accumulator that starts as
`0` and receives floats, branches binding each, a helper called with both,
and a list holding both."""


def _shortest(costs: list[int]) -> None:
    best = float("inf")
    for cost in costs:
        if cost < best:
            best = cost
    print(best, best == float("inf"))
    worst = float("-inf")
    for cost in costs:
        worst = max(worst, cost)
    print(worst)
    lowest = float("inf")
    for cost in costs:
        lowest = min(lowest, cost * 2)
    print(lowest)


_shortest([5, 3, 9])
_shortest([])


def total(n: int) -> int | float:
    s = 0
    for i in range(n):
        s += 0.25 * i
    return s


print(total(0), total(1), total(5))
acc = 0
for i in range(4):
    acc = acc + i / 2
print(acc)
count = 0
count += 1
count *= 2.5
print(count)


def pick(flag: bool) -> None:
    if flag:
        v = 1
    else:
        v = 2.5
    print(v, v * 2)


pick(True)
pick(False)


def _scale(x):
    return x * 2


print(_scale(3), _scale(1.5))
out = []
for t in range(3):
    if t == 0:
        out.append(0)
        continue
    bob = float("inf")
    for i in range(4):
        ans = i * t + 1
        if ans < bob:
            bob = ans
    out.append(bob)
out.append(0.5)
print(out, sum(out), max(out), min(out), sorted(out))
xs = [1, 2.5, True, -0.0, 10 ** 15]
print(xs, len(xs), xs[1], xs[-1], 2.5 in xs, 1.0 in xs, 3 in xs, xs.index(2.5))
xs.append(7)
xs[0] = 0.25
print(xs, xs == [0.25, 2.5, True, -0.0, 10 ** 15, 7], sorted(xs, reverse=True))
pairs = {1: 1.5, 2: 3}
print(pairs, pairs[1] + pairs[2])
mixed = (1, 2.5)
print(mixed, mixed == (1, 2.5), 1 if mixed[0] else 0.5)
y = 3
y = 4.5 if y > 2 else y
print(y)
z = 0 or 2.5
print(z)
