"""Attribute-cache semantics: values, methods, shadowing, class changes, __getattr__."""


def keep(f):
    return f


class Point:
    @keep
    def unused(self):
        return 0

    def __init__(self, x, y):
        self.x = x
        self.y = y

    def norm(self):
        return self.x + self.y


class Hooked:
    @keep
    def unused(self):
        return 0

    def __init__(self):
        self.a = 1

    def __getattr__(self, name):
        return "missing:" + name

    def m(self):
        return "m"


class Sub(Point):
    def norm(self):
        return 100


def read_x(p):
    return p.x


def call_norm(p):
    return p.norm()


def read_a(h):
    return h.a


def call_m(h):
    return h.m()


def text(s):
    return s == "abc", s != "abc", "abc" == s, s == "hé", s == ""


def main():
    p = Point(1, 2)
    q = Point(3, 4)
    out = []
    for _ in range(3):
        out.append((read_x(p), read_x(q), call_norm(p), call_norm(q)))
    del q.x
    try:
        read_x(q)
    except AttributeError as e:
        out.append(("err", str(e)))
    q.x = 9
    out.append(read_x(q))
    # Instance shadows the method.
    q.norm = lambda: "shadow"
    out.append((call_norm(p), call_norm(q)))
    # Class attribute added later shadows the instance value? No: a non-data
    # class attribute does not, but a property does.
    Point.x = property(lambda self: "prop")
    out.append(read_x(p))
    del Point.x
    out.append(read_x(p))
    # The compiled method's code replaced in place: the class is unchanged.
    norm = Point.__dict__["norm"]
    saved = norm.__code__
    out.append(call_norm(p))
    ns = {}
    exec("def other(self):\n    return 'swapped'\n", ns)
    norm.__code__ = ns["other"].__code__
    out.append(call_norm(p))
    norm.__code__ = saved
    out.append(call_norm(p))
    # Method replaced on the class.
    Point.norm = lambda self: "new"
    out.append((call_norm(p), call_norm(Sub(1, 1))))
    # Subclass instances with the same names.
    s = Sub(7, 8)
    out.append((read_x(s), call_norm(s)))
    # Many attributes so values are invalidated (dict materialization).
    r = Point(1, 1)
    r.__dict__["zz"] = 3
    for i in range(40):
        setattr(r, "f%d" % i, i)
    out.append((read_x(r), call_norm(r)))
    h = Hooked()
    out.append((read_a(h), call_m(h), read_a(h)))
    del h.a
    out.append(read_a(h))
    h.m = lambda: "inst"
    out.append(call_m(h))
    Hooked.__getattribute__ = lambda self, n: "ga:" + n
    out.append((read_a(h), call_m(Hooked.__new__(Hooked)) if False else read_a(h)))
    out.append([text(v) for v in ("abc", "ab", "abcd", "".join(["a", "bc"]), None, 3, "hé", "".join(["h", "é"]), "", "abc\ud800", "abé")])
    for line in out:
        print(line)


main()
