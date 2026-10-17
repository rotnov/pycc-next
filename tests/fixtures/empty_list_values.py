"""Empty list displays whose element type nothing observes: printed,
measured, tested, sorted, concatenated or repeated, natively."""

xs = [3, 1, 2]
print([], len([]), bool([]), sorted([]), [] * 3, list(reversed([])))
print(xs + [], str([]), repr([]), sum([]), any([]), all([]))
total = 0
for v in xs + []:
    total += v
print(total, max(xs + []), len([] + [] + xs))


def _pick(flag: bool) -> list[int]:
    if flag:
        return []
    return [1]


print(_pick(True), _pick(False), [_pick(True), []])


def _count() -> int:
    return len([])


print(_count())
