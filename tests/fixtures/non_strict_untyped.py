# Non-strict mode (#3): untyped code compiled with `--non-strict` must match
# CPython. Unannotated public parameters and returns are CPython objects.


def add(a, b):
    return a + b


def scale(x, k=2):
    return x * k


def typed_sq(n: int) -> int:
    return n * n


def calls_typed(x):
    return typed_sq(x) + 1


def typed_calls_untyped(n: int) -> int:
    return add(n, 1)


def _helper(a):
    return a * 3


def uses_helper(v):
    return _helper(v)


def build(a, b):
    xs = [a, b, a + b]
    d = {"a": a, "sum": a + b}
    d["b"] = b
    t = (a, 1, "s")
    return xs, d, t


def mixed():
    return [1, "two", 3.0], {1: "one", "two": 2}


def total(xs):
    t = 0
    for x in xs:
        t += x
    return t


def count_up(n):
    i = 0
    out = []
    while i < n:
        out.append(i * 2)
        i += 1
    return out


def extend(xs, more):
    alias = xs
    xs += more
    return alias is xs, alias


def bump(d, k):
    d[k] += 1
    del d["gone"]
    return d


def describe(v):
    if v > 10:
        return "big"
    return "small"


def safe_div(a, b):
    try:
        return a / b
    except ZeroDivisionError:
        return "div0"


def unary(v):
    return -v, +v, ~v


print(add(1, 2), add("x", "y"), add(1.5, 2), add(2, True))
print(scale(3), scale("ab"), scale(1.5, 3), scale([0], 3))
print(add(7, 2) // 2, add(7, 2) % 4, add(2, 0) ** 10, add(6, 0) / 4)
print(add(1, 2) << 3, add(255, 0) & 15, add(1, 0) | 6, add(5, 0) ^ 1)
print(calls_typed(4), typed_calls_untyped(5))
print(uses_helper(2), uses_helper("ab"))
print(build(1, 2))
print(mixed())
print(total([1, 2, 3]), total([1.5, 2]), total([True, 2]) if False else "skip")
print(count_up(4))
print(extend([1], [2, 3]), extend((1,), (2,)))
print(bump({"a": 1, "gone": 0}, "a"))
print(describe(11), describe(3))
print(safe_div(7, 2), safe_div(1, 0))
print(unary(5), -add(0.5, 0))
r = add(10, 5)
r += 1
r -= add(1, 1)
print(r)
x = 1
x = "now a str"
print(x)
try:
    add(1, "a")
except TypeError as e:
    print("TypeError:", e)
try:
    calls_typed("no")
except TypeError:
    print("TypeError from a typed parameter")
try:
    print(add(1, 0) / 0)
except ZeroDivisionError as e:
    print("ZeroDivisionError:", e)
try:
    bump({}, "a")
except KeyError as e:
    print("KeyError:", e)
print(add(1, "a"))
