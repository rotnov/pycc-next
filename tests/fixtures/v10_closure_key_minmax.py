# v0.10: `min`/`max` compare each element right after computing its key,
# so a nested function key with an effect (here it changes a key it
# returned earlier) sees CPython's order of calls; the call is not
# rewritten into keys-first form.


def smallest(xs):
    boxes = []

    def key(v):
        box = [v]
        boxes.append(box)
        if len(boxes) == 3:
            boxes[0][0] = 0
        return box

    return min(xs, key=key)


def largest(xs):
    seen = []

    def key(v):
        seen.append(v)
        print("key", v, len(seen))
        return v

    return max(xs, key=key)


def pure_key(xs, base):
    base = base + 0

    def key(v):
        return abs(v - base)

    return min(xs, key=key), max(xs, key=key), sorted(xs, key=key)


print(smallest([2, 1, 3]))
print(largest([2, 3, 1]))
print(pure_key([5, -2, 3, 9], 2))
