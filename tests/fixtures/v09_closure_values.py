# v0.9 accept 1: a nested function with unannotated parameters used as a
# value -- passed to `map`, returned, stored, called by CPython code -- is a
# native closure over the enclosing variables it reads (non-strict mode: its
# parameters are CPython objects).
import functools


class Node:
    def __init__(self, text):
        self.text = text

    def render(self, prefix, upper):
        return prefix + (self.text.upper() if upper else self.text)


def render_all(parts, prefix, upper):
    def get_text(node):
        if type(node) is str:
            return node
        return node.render(prefix, upper)

    return "".join(list(map(get_text, parts)))


def lookup(table, name):
    if name not in table:
        raise KeyError(name)
    return table[name]


def make_resolver(strict):
    if strict:
        return lookup

    def resolve(table, name):
        try:
            return lookup(table, name)
        except KeyError:
            return ""

    return resolve


def make_counter(start):
    def step(by):
        return start + by

    return step


def run():
    parts = ["a", Node("b"), "c", Node("d")]
    print(render_all(parts, "<", False), render_all(parts, ">", True), render_all([], "", True))
    loose = make_resolver(False)
    print(repr(loose({"x": 1}, "x")), repr(loose({}, "y")))
    strict = make_resolver(True)
    try:
        strict({}, "z")
    except KeyError:
        print("KeyError")
    step = make_counter(10)
    print(step(5), step(2.5), sorted([3, 1, 2], key=make_counter(0)))
    print(functools.reduce(lambda a, b: a + b, map(make_counter(1), [1, 2, 3])))
    print(callable(step), callable(loose))
    try:
        step("x")
    except TypeError as e:
        print("TypeError", e)


run()
