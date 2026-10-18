"""Nested `def` headers (decorators, defaults) naming the enclosing function,
its locals, module globals, and names that are both."""

import functools

TAG = "g"


def outer(n):
    @functools.wraps(outer)
    def inner(k):
        return k + n

    return inner


def recurse(n):
    # The body names the enclosing function: the module global.
    def step(k):
        return k if k <= 0 else recurse(k - 1)(k - 1)

    return step


def local_deco(n):
    def deco(func):
        func.mark = n
        return func

    @deco
    def inner(k):
        return k * n

    return inner


def global_deco(n):
    @functools.lru_cache(maxsize=None)
    def inner(k):
        return (TAG, k, n)

    return inner


def shadowed(TAG):
    # `TAG` is a module global and an enclosing local: the local wins.
    def deco(func):
        func.tag = TAG
        return func

    @deco
    def inner(k=TAG):
        return k

    return inner


def defaults(n, scale=2):
    base = n * scale

    def inner(k=base, m=n, tag=TAG):
        return (k, m, tag)

    return inner


def own_name_default(n):
    def inner(k=n, f=own_name_default):
        return k

    return inner



print(outer(1)(5), outer(1).__name__, outer(1).__wrapped__ is outer)
print(recurse(3)(3))
f = local_deco(3)
print(f(2), f.mark, f.__qualname__)
g = global_deco(4)
print(g(1), g(1) is g(1), g.__wrapped__.__qualname__)
h = shadowed("local")
print(h(), h.tag, h("x"))
print(defaults(3)(), defaults(3, 5)(1), defaults.__name__)
o = own_name_default(7)
print(o(), o.__defaults__[1] is own_name_default)
try:
    outer(1)("x")
except TypeError as e:
    print("TypeError", e)
