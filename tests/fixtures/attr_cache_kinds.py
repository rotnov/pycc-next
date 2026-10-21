"""Attribute-cache semantics: slots, properties, builtin methods, polymorphic sites."""


def keep(f):
    return f


class Slotted:
    __slots__ = ("a", "b")

    @keep
    def unused(self):
        return 0

    def __init__(self, a):
        self.a = a

    def get(self):
        return self.a


class WithProp:
    @keep
    def unused(self):
        return 0

    def __init__(self, v):
        self._v = v
        self.calls = 0

    @property
    def v(self):
        self.calls += 1
        return self._v


class HookProp:
    @keep
    def unused(self):
        return 0

    @property
    def p(self):
        raise AttributeError("inner")

    def __getattr__(self, name):
        return "fallback:" + name


class A:
    @keep
    def unused(self):
        return 0

    def __init__(self):
        self.a = "A.a"

    def get(self):
        return "A.get"


class B:
    @keep
    def unused(self):
        return 0

    def __init__(self):
        self.x = 0
        self.a = "B.a"

    def get(self):
        return "B.get"


def read_a(o):
    return o.a


def read_b(o):
    return o.b


def read_v(o):
    return o.v


def read_p(o):
    return o.p


def call_get(o):
    return o.get()


def bound_get(o):
    f = o.get
    return f()


def dget(d):
    return d.get("k"), d.get("z")


def joined(s):
    return s.join(["x", "y"])


def main():
    out = []
    s = Slotted(1)
    for _ in range(3):
        out.append((read_a(s), call_get(s), bound_get(s)))
    try:
        read_b(s)
    except AttributeError as e:
        out.append(("err", type(e).__name__))
    s.b = 5
    out.append(read_b(s))
    del s.a
    try:
        read_a(s)
    except AttributeError as e:
        out.append(("err", type(e).__name__))
    w = WithProp(7)
    for _ in range(3):
        out.append(read_v(w))
    out.append(w.calls)
    WithProp.__dict__["v"].__init__(lambda self: "reinit")
    out.append(read_v(w))
    WithProp.v = property(lambda self: "replaced")
    out.append(read_v(w))
    h = HookProp()
    for _ in range(2):
        out.append(read_p(h))
    objs = [A(), B(), A(), B(), Slotted(9)]
    for o in objs:
        out.append((read_a(o), call_get(o), bound_get(o)))
    for _ in range(2):
        out.append(dget({"k": 1}))
    class D(dict):
        def get(self, k, default=None):
            return "D.get"
    out.append(dget(D()))
    out.append(joined("-"))
    out.append(joined(","))
    try:
        dget(3)
    except AttributeError as e:
        out.append(("err", str(e)))
    for line in out:
        print(line)


main()
