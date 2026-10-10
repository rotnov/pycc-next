# Non-strict mode (#3) across the `--ext` boundary: an untyped module
# imported by CPython must behave exactly like its source.


def add(a, b):
    return a + b


def typed_sq(n: int) -> int:
    return n * n


def calls_typed(x):
    return typed_sq(x) + 1


def total(xs):
    t = 0
    for x in xs:
        t += x
    return t


def record(d, k, v):
    d[k] = v
    return d


def forget(d, k):
    del d[k]
    return d


def pair(a, b):
    return [a, b], {"a": a, "b": b}, (a, b)


def extend(xs, more):
    xs += more
    return xs


def ratio(a, b):
    return a / b
