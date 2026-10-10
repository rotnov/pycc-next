# Oracle fixture for the inline fast paths: instance slots, float `/` and
# `**` (including the exact `x ** 0.5` shortcut), and the smallint version
# of `for ... in range(...)`. Every case also exercises the slow path the
# fast path falls back to.


class Body:
    def __init__(self, x: float, m: float, n: int, name: str) -> None:
        self.x = x
        self.m = m
        self.n = n
        self.name = name


class Late:
    late: int
    copy: int

    def __init__(self, early: int) -> None:
        self.early = early
        if early > 0:
            self.late = early * 2
        self.copy = self.late
        self.late = early * 3


def slots() -> None:
    b = Body(1.5, 2.0, 3, "a")
    b.x += b.m * 2.0
    b.n = b.n * 10**20
    b.n = b.n + 1
    b.name = b.name + "bc"
    b.name = "d"
    print(b.x, b.m, b.n, b.name)
    c = b
    c.x = -0.0
    print(b.x, c.x)
    ok = Late(1)
    print(ok.early, ok.late, ok.copy)
    try:
        missing = Late(0)
        print(missing.late)
    except AttributeError as e:
        print("AttributeError", e)


def divide(a: float, b: float) -> None:
    try:
        print(a / b)
    except ZeroDivisionError as e:
        print("ZeroDivisionError", e)


def divisions() -> None:
    divide(1.0, 3.0)
    divide(-7.5, 2.0)
    divide(1.0, 0.0)
    divide(1.0, -0.0)
    divide(0.0, 0.0)
    inf = 1e308 * 10.0
    nan = inf - inf
    divide(1.0, nan)
    divide(inf, inf)
    divide(1.0, inf)
    divide(1e308, 1e-308)
    n = 3
    print(1.0 / n, n / 2.0, (n + 1) / 0.5)
    big = 10**400
    try:
        print(big / 2.0)
    except OverflowError as e:
        print("OverflowError", e)
    try:
        print(2.0 / big)
    except OverflowError as e:
        print("OverflowError", e)
    try:
        print(1.5 + big)
    except OverflowError as e:
        print("OverflowError", e)


def show(v: float) -> None:
    # Prints a positive finite `v` as a mantissa in [1, 2) and a binary
    # exponent: exact (scaling by two never rounds a normal double), and
    # within the float formatting pycc supports.
    e = 0
    if v > 0.0 and v < 1e308 * 10.0:
        while v >= 2.0:
            v = v * 0.5
            e += 1
        while v < 1.0:
            v = v * 2.0
            e -= 1
    print(v, e)


def power(a: float, b: float) -> None:
    try:
        show(a**b)
    except ZeroDivisionError as e:
        print("ZeroDivisionError", e)
    except OverflowError:
        print("OverflowError")


def half(x: float) -> None:
    try:
        show(x**0.5)
    except ZeroDivisionError as e:
        print("ZeroDivisionError", e)


def powers() -> None:
    power(2.0, 10.0)
    power(2.0, -2.0)
    power(10.0, 0.0)
    power(1.0, 1e300)
    power(0.0, 0.0)
    power(0.0, 2.0)
    power(0.0, -1.0)
    power(1e300, 2.0)
    power(2.0, -1075.5)
    power(-2.0, 3.0)
    power(-2.0, -1.0)
    inf = 1e308 * 10.0
    power(inf, 0.5)
    power(-inf, 0.5)
    power(inf - inf, 0.5)
    power(3.0, inf)
    power(0.5, -inf)
    half(inf)
    half(-inf)
    half(inf - inf)
    edges = [0.0, -0.0, 1.0, 4.0, 2.0, 1e-320, 5e-324, 1e-300, 2.0**-900, 2.0**1000, 1.7976931348623157e308]
    for x in edges:
        half(x)
    # Inputs where glibc's `pow(x, 0.5)` differs from `sqrt(x)`.
    hard = [
        509.2356092951321, 603.524516616118, 78.05969691615098, 904.2357355393807,
        372.1876013955717, 903.5627585581317, 301.6958497980386, 973.0861437009121,
        737.6161558655812, 721.2697698144792, 23.773366838381047, 193.44698927532093,
        356.30377828561507, 372.0468635400687, 768.0588714708566, 936.0244498196417,
        681.4299918483484, 313.61211565705196, 207.41771697453808, 611.865517571099,
        958.1144113136816, 674.6222052120607, 442.62380912742645, 348.79741305838303,
        713.0304894477758, 275.47433903862, 776.8346177105136, 123.81345632703268,
        254.44020979868304, 132.88939064626797, 196.47083334653956, 196.58525475020727,
        873.5697768171749, 525.4984180945058, 220.51937425357093, 509.6175627614005,
        533.440616739404, 561.8048357778774, 252.58254422315795, 848.4491051474448,
        2.4190803557352228e212, 7.189095980031873e161, 3.2185674393909916e-72,
        2.2408306964980143e-188, 2.8779226780156256e144, 5.169348431754288e107,
        7.615418580552734e-134, 7.288753946878289e81, 5.955203051030863e-232,
        5.915961852923378e-24, 1.4232111425761015e169, 2.205261199102286e244,
        1.2947671007212086e46, 1.678264759424437e-181, 2.4886955085088704e17,
        4.95394478471127e283, 2.6081214336686954e131, 0.00010550196829928202,
        1.797978419958129e206, 7.401585632570906e-228,
    ]
    for x in hard:
        show(x**0.5)
    # A sweep over ordinary values, most of which take `sqrt`.
    x = 0.37
    for _ in range(400):
        show(x**0.5)
        x = x * 1.0371 + 0.013


def ranges() -> None:
    i = -1
    total = 0
    for i in range(10):
        total += i * i
    print(total)
    for i in range(10, 0, -3):
        print(i)
    for i in range(5, 5):
        print("never")
    for i in range(-3, 3, 2):
        print(i)
    print(i)
    big = 2**62
    for i in range(big - 3, big + 2):
        print(i)
    for i in range(big + 2, big - 3, -2):
        print(i)
    for i in range(2**70, 2**70 + 2):
        print(i)
    for i in range(-big - 2, -big + 2):
        print(i)
    for i in range(True, 4):
        print(i)
    for i in range(0, 7, 1 << 40):
        print(i)
    n = 6
    for i in range(1, n, 2):
        if i == 3:
            continue
        print(i, i + 2**62)
    for i in range(n):
        if i == 4:
            break
    print("broke at", i)
    for i in range(n):
        i = i * 100
    print(i)
    for i in range(3):
        print(i)
    else:
        print("else ran")
    for i in range(3):
        if i == 1:
            break
    else:
        print("not printed")
    print(first_over(7, 20))
    print(first_over(70, 20))
    nested = 0
    for a in range(4):
        for b in range(a, 4):
            nested += a * b
    print(nested)
    try:
        for i in range(0, 3, 0):
            print("never")
    except ValueError as e:
        print("ValueError", e)


def bool_lists() -> None:
    flags = [True] * 5
    flags[1] = False
    flags[-1] = 3 > 4
    flags.append(True)
    flags.insert(0, False)
    print(flags, len(flags), flags[2], flags[-2])
    print(flags.pop(), flags.pop(0), flags)
    print(True in flags, flags[1:4], flags[::2], flags + [False])
    print([f for f in flags if f], [not f for f in flags])
    count = 0
    for f in flags:
        if f:
            count += 1
    print(count)
    grid = [[False] * 3, [True, False]]
    row = grid[0]
    row[2] = True
    print(grid, grid[0][2])
    sieve = [True] * 50
    sieve[0] = False
    sieve[1] = False
    i = 2
    while i * i < 50:
        if sieve[i]:
            j = i * i
            while j < 50:
                sieve[j] = False
                j += i
        i += 1
    print([k for k in range(50) if sieve[k]])
    try:
        print(flags[10])
    except IndexError as e:
        print("IndexError", e)
    try:
        flags[-10] = True
    except IndexError as e:
        print("IndexError", e)
    empty: list[bool] = []
    print(empty, [True] * 0, [False] * -2)


def floor_mix(a: int, b: int) -> int:
    return (a + b) * (a + b + 1) // b + a % b


def inverse(x: float) -> float:
    return 1.0 / x


def shifted(a: int, n: int) -> int:
    return (a << n) >> 1


def leaves() -> None:
    total = 0.0
    for i in range(1, 30):
        for j in range(1, 30):
            total += inverse(float(floor_mix(i, j)))
    print(total * 1000.0)
    print(floor_mix(-7, 3), floor_mix(2**70, 3), shifted(3, 70), shifted(-5, 2))
    for b in range(-1, 2):
        try:
            print(floor_mix(5, b))
        except ZeroDivisionError as e:
            print("ZeroDivisionError", e)
    try:
        print(inverse(0.0))
    except ZeroDivisionError as e:
        print("ZeroDivisionError", e)
    try:
        print(shifted(1, -1))
    except ValueError as e:
        print("ValueError", e)
    n = 0
    for k in range(5):
        try:
            n += 10 // (k - 2)
        except ZeroDivisionError:
            n += 1000
    print(n)


def first_over(limit: int, n: int) -> int:
    for i in range(n):
        if i * i > limit:
            return i
    return -1


slots()
divisions()
powers()
ranges()
bool_lists()
leaves()
