# The module under test of tests/object_operand_pin.rs: every operation
# below runs CPython code on module globals (and on `box.o`, an attribute
# of a compiled instance a rebind of `box` releases), once in the module
# body and once in `probe`, a function the host calls.
import object_operand_pin_stub as s


class Box:
    def __init__(self, o: object) -> None:
        self.o = o


def probe() -> None:
    print(g == h)
    print(g + h)
    print(g + box.o)
    print(-g)
    print(g.missing)
    print(g.method())
    print(g(1))
    print(len(g))
    print(g[0])
    for x in g:
        print(x)
    print(f"{g}")
    print(d[k])
    print(k in d)
    d[k] = v
    print(len(d))
    print(lst)


def get_h() -> object:
    return h


def get_d() -> object:
    return d


g: object = s.Nester("g")
h: object = s.Item("h")
box = Box(s.Item("boxed"))
d: object = s.make_dict()
k: object = s.Key("k")
v: object = s.Item("value")
lst: object = s.make_list()
print(g == h)
print(g + h)
print(g + box.o)
print(-g)
print(g.missing)
print(g.method())
print(g(1))
print(len(g))
print(g[0])
for x in g:
    print(x)
print(f"{g}")
print(d[k])
print(k in d)
d[k] = v
print(len(d))
print(lst)
