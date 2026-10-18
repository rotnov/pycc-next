"""Nested `def` headers in methods: a decorator naming the method's own
name (the module global, not the method) and reading the method's locals."""

import functools


class C:
    def method(self, n):
        @functools.wraps(method)
        def inner(k=n):
            return k

        return inner

    def again(self, n):
        @functools.wraps(again)
        def inner(k):
            return (k, n)

        return inner

    def plain(self, n):
        # Defaults naming the method (the module global) and a local.
        def inner(k=n, f=plain):
            return (k, f(k))

        return inner


def plain(x):
    return -x


def method(x):
    return x


def again(x):
    return x


m = C().method(9)
print(m(), m.__name__, m.__qualname__, m.__wrapped__.__qualname__)
a = C().again(2)
print(a(1), a.__name__, a.__wrapped__.__qualname__, a.__qualname__)
p = C().plain(4)
print(p(), p(1), p.__qualname__, p.__name__)
