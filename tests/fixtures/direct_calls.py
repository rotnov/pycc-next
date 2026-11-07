"""Calls between compiled bodies in generic (unannotated) code
(`crates/pycc_codegen/src/direct_call.rs`): a call whose callee is the
forwarder of a lifted body, binding its parameters in order, runs the body
directly; every other call -- a rebound name, a replaced `__code__`, a
keyword out of order, a default left out, `*args`, an override -- goes
through the callee as CPython calls it, with the same result.
"""

import sys
import traceback


def walk(node, depth=0):
    if isinstance(node, dict):
        return {key: walk(value, depth=depth + 1) for key, value in node.items()}
    if isinstance(node, list):
        return [walk(item, depth + 1) for item in node]
    return (node, depth)


def skip(src, pos, *, until, strict):
    while pos < len(src) and src[pos] != until:
        if strict and src[pos] == "!":
            raise ValueError(f"stray ! at {pos}")
        pos += 1
    return pos


def first(a, b, /):
    return a


def spread(*args):
    return len(args)


def boom(n):
    if n == 0:
        raise KeyError("bottom")
    return boom(n - 1)


def deep(n):
    return deep(n + 1)


def replaced(x):
    return ("old", x)


def other(x):
    return ("new", x)


def target(x):
    return ("target", x)


class Gen:
    def sort(self, value, parent_key=None):
        if isinstance(value, dict):
            out = {}
            keys = value.keys()
            if parent_key not in ("properties", "default"):
                keys = sorted(keys)
            for key in keys:
                out[key] = self.sort(value[key], parent_key=key)
            return out
        if isinstance(value, list):
            return [self.sort(item, parent_key) for item in value]
        return value

    def twice(self, x):
        return self.half(x) * 4

    def half(self, x):
        return x // 2


class Sub(Gen):
    pass


def calls():
    out = []
    out.append(walk({"a": [1, {"b": 2}], "c": "x"}))
    out.append(skip("abc;def", 0, until=";", strict=False))
    out.append(skip("abc;def", 1, until=";", strict=True))
    try:
        skip("a!c;", 0, until=";", strict=True)
    except ValueError as exc:
        out.append(("ValueError", str(exc)))
    out.append(skip("abc;def", until=";", strict=False, pos=2))
    out.append(first(1, 2))
    out.append(spread(1, 2, 3))
    out.append(walk([1, 2]))
    g = Gen()
    out.append(g.sort({"z": 1, "properties": {"y": 2, "x": 3}, "a": [{"d": 1, "c": 2}]}))
    out.append(g.twice(10))
    out.append(Sub().twice(9))
    return out


def tracebacks():
    try:
        boom(3)
    except KeyError as exc:
        frames = [(frame.name, frame.lineno) for frame in traceback.extract_tb(exc.__traceback__)]
        return ("KeyError", str(exc), frames)


def recursion():
    try:
        deep(0)
    except RecursionError:
        return "RecursionError"


def call_replaced():
    return replaced(1)


def call_target():
    return target(2)


def half_of(g):
    return g.twice(8)


def main():
    for row in calls():
        print(row)
    print(tracebacks())
    print(recursion())
    print(call_replaced())
    replaced.__code__ = other.__code__
    print(call_replaced())
    print(call_target())
    global target
    saved = target
    target = other
    print(call_target())
    target = saved
    print(call_target())
    g = Gen()
    print(half_of(g))
    Gen.half = lambda self, x: -x
    print(half_of(g))
    print(sys.getrecursionlimit() > 0)


if __name__ == "__main__":
    main()
