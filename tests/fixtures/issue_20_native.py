# Issue #20: fallback code of a non-strict build reaches compiled module
# globals by a string (`eval`, `exec`, `globals()`, module-level `locals()`,
# `vars()`, `dir()`, the module object), reading and rebinding them as
# CPython does.
import math
import sys
from typing import Optional


class Point:
    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y

    def norm1(self) -> int:
        return abs(self.x) + abs(self.y)


p = Point(3, -4)
count = 5
ratio = 0.5
label = "hi"
items = [1, 2]
table = {"a": 1}
pair = (1, "x")
maybe: Optional[int] = None
total = 0
for i in range(3):
    total += i


def _scaled(a: int) -> int:
    return a * 10


def untyped(a):
    return a + 1


def read_count() -> int:
    return count


def read_point() -> int:
    return p.norm1()


def read_items() -> int:
    return len(items)


def read_label() -> str:
    return label


def read_maybe() -> int:
    return -1 if maybe is None else maybe


def via_lambda() -> int:
    get = lambda: eval("count + total")
    return get()


def reads():
    print(eval("p").x, eval("p.norm1()"), eval("count + 1"), eval("ratio * 4"))
    print(eval("label"), eval("items"), eval("table"), eval("pair"), eval("maybe"), eval("total"), eval("i"))
    print(eval("_scaled(2)"), eval("untyped(4)"), eval("math.floor(2.5)"), eval("Point(1, 2).norm1()"))
    print(eval("Optional"), type(eval("p")).__name__)
    exec("print('exec sees', p.y, count, label, items)")
    print(sorted(k for k in globals() if not k.startswith("__"))[:6])
    print(globals()["count"], globals()["p"].x)
    try:
        eval("missing_name")
    except NameError as e:
        print("NameError:", e)
    try:
        eval("count", {})
    except NameError as e:
        print("NameError:", e)
    print(eval("count", {"count": 99}))


def writes():
    globals()["count"] = 7
    exec("global label\nlabel = 'rebound'")
    globals()["p"] = Point(10, 20)
    globals()["items"] = [4, 5, 6]
    exec("global maybe\nmaybe = 12")


def via_module():
    # `eval` keeps this function in CPython; reading the globals through the
    # module object, it may rebind any of them as well.
    mod = sys.modules[__name__]
    print(eval("0"), mod.p.x, getattr(mod, "count"), mod.label)
    mod.p = Point(7, 7)
    setattr(mod, "ratio", 2.5)


reads()
print(via_lambda())
writes()
print(read_count(), read_label(), read_point(), read_items(), read_maybe())
print(count, label, p.x, items, maybe)
exec("count = count + 100")
print(read_count(), count)
via_module()
print(read_point(), p.x, ratio)
print(sorted(k for k in locals() if not k.startswith("__")))
print(sorted(k for k in dir() if not k.startswith("__")) == sorted(k for k in vars() if not k.startswith("__")))
exec("raise ValueError(p.norm1())")
