# Non-strict mode: a comprehension of CPython objects over a native source,
# or over a name bound to a CPython object, builds CPython's own list or set;
# a fresh native list operand of an operator on an object and a dict, set or
# tuple display argument of an object operation cross as CPython copies.
import json

x = json.loads("2")
xs = [i * x for i in range(3)]
print(xs, type(xs).__name__)
ys = {x + i % 2 for i in range(5) if i != 3}
print(ys)

c = json.loads('{"1": 5, "3": 7}')
cl = [c.get(str(i), 0) for i in range(1, 5)]
print(cl, sum(cl))


def f(n):
    o = json.loads("[1, 2, 3]")
    out = [o[i] * 2 for i in range(n) if i > 0]
    out.append(9)
    return out


print(f(3))
try:
    print(f(5))
except IndexError as e:
    print("IndexError", e)

names = ["a", "b"]
objs = [json.loads('{"k": "' + s + '"}') for s in names]
print(objs, [o["k"] for o in objs])
for o in objs:
    print(o["k"])


def g():
    objs = json.loads('[{"k": 1}, {"k": 2}]')
    ks = [o["k"] + 1 for o in objs]
    return ks


print(g())

n = json.loads("3")
tree = [0] * (n + 1)
print(tree, [1] + json.loads("[2]"), json.loads("[3]") + [4, 5])

d = json.loads("{}")
d["a"] = {"x": 1, "y": 2}
d["b"] = (1, "s")
d["c"] = {3, 4}
print(d)
