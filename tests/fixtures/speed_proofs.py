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


def tri(i: int, j: int) -> int:
    return (i + j) * (i + j + 1) // 2 + i + 1


def mix(a: int, b: int) -> int:
    return (a * 3 - b) // 7 + (a - b * b) % 5 - a % 3


def trees() -> None:
    for i in range(-4, 5):
        for j in range(-3, 4):
            print(tri(i, j), mix(i, j), end=" ")
        print()
    seeds = [0, 1, -1, 2**61 - 1, -(2**61), 2**31, -(2**31), 3 * 2**59]
    for sa in seeds:
        for sb in seeds:
            for m in range(1, 5):
                a = sa * m
                b = sb * (5 - m) - m
                print(tri(a, b), mix(a, b), tri(a * 10**20, b), mix(a, b - 10**25))
    t: int = True
    f: int = False
    print(tri(t, f), mix(t, t), tri(f, 5))


def fib(n: int) -> int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


def fib_float(x: float, n: int) -> float:
    if n == 0:
        return x
    return fib_float(x * 1.5 - 0.25, n - 1)


def no_throw() -> None:
    print(fib(20), fib(-3), fib(0))
    print(fib_float(2.0, 30))
    x = 2**100
    print(fib(2) + x)


classes()
loop_vars()
global_loop()
calls()
trees()
no_throw()


def after(n: int) -> int:
    return n


print(after(2), call_after())
