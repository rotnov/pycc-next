# `d.get(k)` with no default is an `Optional` of the value type; an
# `Optional[int|float|bool]` compares with `== None`/`!= None`, narrows on
# them, and renders as `None` or its payload.


def count_firsts(xs: list[int]) -> list[int]:
    seen = {}
    order = []
    for x in xs:
        if seen.get(x) == None:
            seen[x] = 0
            order.append(x)
        seen[x] += 1
    return [seen[x] for x in order]


print(count_firsts([3, 1, 3, 2, 1, 3]))
d = {1: 5, 2: 0}
for k in [1, 2, 3]:
    w = d.get(k)
    print(w is None, w, f"[{w}]", str(w))
    if w != None:
        print("present", w + 1)
    else:
        print("absent", k)
scores = {"a": 1.5, "b": -2.0}
for name in ["a", "b", "c"]:
    s = scores.get(name)
    print(name, s, s == None)
flags = {1: True, 2: False}
for k in [1, 2, 3]:
    print(k, flags.get(k))
r = d.get(9)
print(r, r == None, r != None, None == r)
