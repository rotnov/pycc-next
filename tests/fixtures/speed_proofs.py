# Oracle fixture for the v0.2 speed proofs: every check the compiler drops
# on a proof must still fire, with CPython's exception, wherever the proof
# does not hold.


def early(n: int) -> int:
    return n + 1


class Point:
    def __init__(self, x: float, y: float) -> None:
        self.x = x
        self.y = y


class Partial:
    def __init__(self, a: int, flag: bool) -> None:
        self.a = a
        if not flag:
            return
        self.b = a * 2


class Base:
    def __init__(self, v: int) -> None:
        self.v = v


class Derived(Base):
    def __init__(self, w: int) -> None:
        self.w = w


class Inherits(Base):
    pass


def show_v(b: Base) -> None:
    try:
        print(b.v)
    except AttributeError as err:
        print("AttributeError", err)


class Escapes:
    def __init__(self, v: int) -> None:
        self.first = v
        note(self)
        self.second = v + 1


def note(e: Escapes) -> None:
    try:
        print("seen", e.first, e.second)
    except AttributeError as err:
        print("AttributeError", err)


class SelfRead:
    def __init__(self, v: int) -> None:
        self.a = v
        self.b = self.a + 1


class Rebind:
    def __init__(self, v: int, other: Point) -> None:
        self.x = float(v)
        self.y = other.y


def points(ps: list[Point]) -> float:
    total = 0.0
    for p in ps:
        p.x += 1.0
        total += p.x * p.y
    return total


def classes() -> None:
    ps = [Point(1.0, 2.0), Point(3.0, 4.0)]
    print(points(ps), points(ps))
    ok = Partial(3, True)
    print(ok.a, ok.b)
    no = Partial(4, False)
    print(no.a)
    try:
        print(no.b)
    except AttributeError as err:
        print("AttributeError", err)
    no.b = 9
    print(no.b)
    d = Derived(5)
    print(d.w)
    try:
        print(d.v)
    except AttributeError as err:
        print("AttributeError", err)
    show_v(Base(1))
    i = Inherits(7)
    print(i.v)
    e = Escapes(10)
    print(e.first, e.second)
    s = SelfRead(4)
    print(s.a, s.b)
    r = Rebind(2, Point(0.5, 0.25))
    print(r.x, r.y)


def loop_vars() -> None:
    j = 10**30
    for j in range(5, 5):
        print("never")
    print(j)
    for j in range(3):
        pass
    print(j)
    k = 2**70
    for k in range(2, 4):
        print(k)
    print(k)
    m = 2**80
    for m in range(4, 0, -2):
        print(m)
    print(m)
    t = 0
    for q in range(3):
        for r in range(q):
            t += q * r
    print(t)


g = 0


def bump() -> None:
    global g
    g = g + 100


def global_loop() -> None:
    global g
    for g in range(3):
        bump()
        print(g)
    print(g)


def calls() -> None:
    print(early(41))
    print(late_def(1))


def late_def(n: int) -> int:
    return n * 3


def call_after() -> int:
    return after(1)


classes()
loop_vars()
global_loop()
calls()


def after(n: int) -> int:
    return n


print(after(2), call_after())
