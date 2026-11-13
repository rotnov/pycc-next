"""Method calls from compiled code into compiled methods that leave later
parameters to their defaults: positional defaults, keyword-only defaults,
and both, with the leading parameters passed positionally or by keyword in
order. The defaults are the ones the function holds when it is called: a
`__kwdefaults__` dict changed in place, a `__defaults__` tuple replaced, a
keyword-only default deleted or the dict dropped, and a mutable default
shared between calls keep CPython's results and errors, from compiled code
and from bytecode alike.
"""

from typing import Any


class K(metaclass=type):
    def __init__(self, x: Any) -> None:
        self.x = x

    def get(self, key: Any, *, lists: Any = True) -> Any:
        return ("get", self.x, key, lists)

    def f(self, a: Any, b: Any = 2, *, c: Any = 3) -> Any:
        return ("f", a, b, c)

    def acc(self, item: Any, *, into: Any = []) -> Any:
        into.append(item)
        return len(into)

    def defaulted(self, n: Any) -> Any:
        out = []
        for i in range(n):
            out.append(self.get(i))
            out.append(self.f(i))
        return out

    def shapes(self) -> Any:
        return [
            self.get(1),
            self.get(key=2),
            self.get(3, lists=False),
            self.f(1),
            self.f(1, 5),
            self.f(1, b=5),
            self.f(a=1),
            self.f(1, c=9),
            self.f(1, 5, c=9),
            self.f(c=9, a=1),
        ]

    def errors(self) -> Any:
        out = []
        for call in (
            lambda: self.f(),
            lambda: self.f(1, 2, 3),
            lambda: self.get(),
            lambda: self.f(1, a=1),
        ):
            try:
                out.append(call())
            except TypeError as e:
                out.append(("TypeError", str(e)))
        return out

    def accumulated(self) -> Any:
        return [self.acc(1), self.acc(2), self.acc(3, into=[])]

    def missing(self) -> Any:
        try:
            return self.f(1)
        except TypeError as e:
            return ("TypeError", str(e))


def main() -> None:
    k = K("x")
    out = k.defaulted(150)
    print(len(out), out[:4], out[-2:])
    print(k.shapes())
    print(k.errors())
    print(k.accumulated(), k.accumulated())
    # Called from bytecode: the forwarder's own trampoline.
    print(k.get(7), k.f(7), k.f(7, c=8))
    # The keyword-only default changed in place, then the positional
    # defaults replaced.
    K.f.__kwdefaults__["c"] = 30
    print(k.shapes()[3:], k.f(1))
    K.f.__defaults__ = (20,)
    print(k.shapes()[3:], k.f(1))
    # A keyword-only parameter left without a default, then the dict gone.
    del K.f.__kwdefaults__["c"]
    print(k.missing(), k.f(1, c=9), k.f(1, 5, c=9))
    K.f.__kwdefaults__ = None
    print(k.missing())
    K.f.__kwdefaults__ = {"c": "restored"}
    K.f.__defaults__ = None
    print(k.missing(), k.f(1, 2))


if __name__ == "__main__":
    main()
