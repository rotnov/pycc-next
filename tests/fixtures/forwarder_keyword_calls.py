"""Keyword method calls from compiled code into compiled methods of a class
CPython creates: the method is loaded with no bound method made, and a call
whose keywords name the parameters in their order goes straight to the
compiled body. Out-of-order keywords, a defaulted parameter left out, a
keyword naming a positional-only parameter, an unknown or repeated keyword,
an instance attribute or a class rebinding shadowing the method, a plain
function stored on the instance, a `staticmethod`, a `classmethod`, a module
object and `**kw` unpacking keep CPython's results and errors.
"""

import types
from typing import Any


def side(x: Any) -> Any:
    return x


class K(metaclass=type):
    def __init__(self, x: Any) -> None:
        self.x = x

    def kw1(self, value: Any, parent_key: Any = None) -> Any:
        return ("kw1", self.x, value, parent_key)

    def kw3(self, a: Any, b: Any, c: Any = "c", d: Any = "d") -> Any:
        return ("kw3", a, b, c, d)

    def posonly(self, a: Any, /, b: Any) -> Any:
        return ("posonly", a, b)

    @staticmethod
    def st(a: Any, b: Any = 0) -> Any:
        return ("st", a, b)

    @classmethod
    def cm(cls, a: Any, b: Any = 0) -> Any:
        return ("cm", cls.__name__, a, b)

    def recurse(self, value: Any, parent_key: Any = None) -> Any:
        if isinstance(value, dict):
            return {k: self.recurse(value[k], parent_key=k) for k in sorted(value)}
        if isinstance(value, list):
            return [self.recurse(v, parent_key=parent_key) for v in value]
        return (parent_key, value)

    def calls(self, n: Any) -> Any:
        out = []
        for i in range(n):
            out.append(self.kw1(i, parent_key=side(i)))
            out.append(self.kw1(side(i), parent_key="p"))
            out.append(self.kw1(value=i))
            out.append(self.kw1(parent_key=i, value="v"))
            out.append(self.kw3(i, b=1))
            out.append(self.kw3(i, 1, c=2, d=3))
            out.append(self.kw3(i, 1, d=3, c=2))
            out.append(self.kw3(i, 1, d=side(4)))
            out.append(self.st(i, b=1))
            out.append(self.cm(i, b=2))
        return out

    def errors(self) -> Any:
        out = []
        for call in (
            lambda: self.posonly(a=1, b=2),
            lambda: self.kw1(1, value=2),
            lambda: self.kw1(1, nope=2),
            lambda: self.kw3(1, b=2, e=3),
            lambda: self.kw1(),
        ):
            try:
                out.append(call())
            except TypeError as e:
                out.append(("TypeError", str(e)))
        try:
            out.append(self.posonly(1, b=2))
        except TypeError as e:
            out.append(("TypeError", str(e)))
        return out

    def unpacked(self, kw: Any) -> Any:
        return [self.kw3(1, **kw), self.kw3(*[1, 2], d=4)]


def on_module(mod: Any) -> Any:
    return mod.f(1, b=2)


def main() -> None:
    k = K("x")
    out = k.calls(150)
    print(len(out), out[:12], out[-10:])
    print(k.recurse({"b": [1, {"c": 2}], "a": 3, "d": {"e": [4, 5]}}))
    print(k.errors())
    print(k.unpacked({"b": 2, "c": 3}))
    # The method shadowed by an instance attribute, then restored.
    k.kw1 = lambda value, parent_key=None: ("shadow", value, parent_key)
    print(k.calls(1)[:4])
    del k.kw1
    print(k.calls(1)[:4])
    # Rebound on the class, then a plain function stored on the instance.
    saved = K.kw3
    K.kw3 = lambda self, a, b, c="C", d="D": ("rebound", a, b, c, d)
    print(k.calls(1)[4:8])
    K.kw3 = saved
    print(k.calls(1)[4:8])
    k.st = lambda a, b=0: ("instance", a, b)
    print(k.calls(1)[8:])
    mod = types.ModuleType("mod")
    mod.f = lambda a, b=0: ("module", a, b)
    print(on_module(mod))


if __name__ == "__main__":
    main()
