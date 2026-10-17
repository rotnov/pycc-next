"""`int` locals whose every value is a proven smallint (`pycc_codegen`'s
`int_range`): arithmetic, comparisons and stores on them skip the tag tests,
overflow checks and bigint releases. Each function mixes such locals with
ones that leave the smallint range, so a wrong proof shows as a wrong value.
"""


def byte_mix(n: int) -> int:
    # `& 0xff`, `>>`, `<<` and `|`/`^` of non-negative values stay bytes.
    acc = 0
    for i in range(n):
        x = (i * 7 + 3) & 0xFF
        y = (x >> 2) | (x << 3) & 0x3FF
        z = (x ^ y) & 0xFF
        acc = (acc + x * y - z) % 1000003
    return acc


def paeth_like(a0: int) -> int:
    # The shapes of PNG's Paeth predictor over values in [0, 255]: every
    # local but `total` is proven (`a0` is masked once).
    total = 0
    base = a0 & 0xFF
    for i in range(500):
        a = (base + i * 37) & 0xFF
        b = (i * 91 + 5) & 0xFF
        c = (a * b) & 0xFF
        p = a + b - c
        pa = abs(p - a)
        pb = abs(p - b)
        pc = abs(p - c)
        if pa <= pb and pa <= pc:
            pr = a
        elif pb <= pc:
            pr = b
        else:
            pr = c
        total += (pr + i) & 0xFF
    return total


def _floors(n: int) -> list[int]:
    # `//` and `%` by a positive divisor floor for negative operands too.
    out: list[int] = []
    for i in range(-n, n):
        q = i // 3
        r = i % 3
        s = (i - 7) // 4
        t = (i * 5) % 7
        out.append(q * 10 + r + s - t)
    return out


def _shifts(n: int) -> list[int]:
    out: list[int] = []
    for k in range(n):
        v = k - 20
        out.append((v >> 1) + (v >> 70) + (v << 3))
    return out


def edges() -> None:
    # Values at and past the smallint boundary (2**62): the locals that
    # leave it are not proven, and their arithmetic promotes to bigints.
    top = 4611686018427387903
    low = -4611686018427387904
    one = 1
    big = top + one
    small = top - one
    wide = top * 2
    neg = low - one
    shifted = one << 61
    doubled = shifted + shifted
    print(top, low, big, small, wide, neg, shifted, doubled, big - top, low // 3, low % 7)


def grows(n: int) -> int:
    # A local that only grows: its range is given up, and it leaves the
    # smallint range on the way.
    x = 1
    for _ in range(n):
        x = x * 3 + 1
    return x % 1000000007


def conditional(n: int) -> int:
    t = 0
    for i in range(n):
        v = i * 2 if i % 2 == 0 else -i
        w = abs(v) - abs(-v) + (v if v > 0 else 0)
        t += w
    return t


def _compares(n: int) -> list[bool]:
    out: list[bool] = []
    for i in range(n):
        a = i & 7
        b = 7 - (i % 8)
        out.append(a < b)
        out.append(a == b)
        out.append(a >= b and a != 3)
    return out


def main() -> None:
    print(byte_mix(300))
    print(paeth_like(17))
    print(paeth_like(-(2**70) - 3))
    print(_floors(8))
    print(_shifts(41))
    edges()
    print(grows(60))
    print(conditional(25))
    print(_compares(12))
    print(10 // 0)


main()
