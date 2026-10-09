import itertools

int_inputs = lambda: tuple(map(int, input().rstrip().split()))
contains = lambda t1, t2: t1[0] <= t2[0] and t1[1] >= t2[1]


def process(b1, b2, left, right, group):
    if not group:
        gen = iter([(0, 0)])
    elif right is None:
        gen = iter([(group[-1][0] - left, 0)])
    elif left is None:
        gen = iter([(0, right - group[0][1])])
    else:
        gen = zip(
            itertools.chain([0], [current[0] - left for current in group]),
            itertools.chain([right - current[1] for current in group], [0]),
        )
    dl, dr = next(gen)
    v = min(b1 + 2 * dl, b2 + dl)
    b1n, b2n = v + dr, v + dr * 2
    for dl, dr in gen:
        v = min(b1 + 2 * dl, b2 + dl)
        b1n, b2n = min(b1n, v + dr), min(b2n, v + dr * 2)
    return b1n, b2n


for t in range(int(input())):
    n, m = int_inputs()
    P = sorted(int_inputs())  # points
    S0 = sorted([int_inputs() for i in range(m)])  # segements
    S1 = []  # if A contains B, remove A won't change result
    for current in S0:
        if S1 and contains(current, S1[-1]):
            continue
        while S1 and contains(S1[-1], current):
            S1.pop()
        S1.append(current)
    # at this moment, all left value and right value of S1 are sorted and unique respectively
    areas = zip([None] + P, P + [None])
    S = [list(next(areas)) + [[]]]
    for current in S1:
        # at any moment, left is None or current[0] > left holds
        left, right, group = S[-1]
        while right is not None and current[0] > right:
            S.append(list(next(areas)) + [[]])
            left, right, group = S[-1]
        if right is not None and current[1] >= right:
            continue
        group.append(current)
    for area in areas:  # remaining
        S.append(list(area) + [[]])
    b1 = b2 = 0
    for left, right, group in S:
        b1, b2 = process(b1, b2, left, right, group)
    print(min(b1, b2))
