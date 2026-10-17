"""Functions whose locals `int_range` proves smallints, for an `--ext` module
called from CPython with arguments of other types and magnitudes
(`int_ranges_driver.py`)."""


def mix(a0: int, n: int) -> int:
    total = 0
    for i in range(n):
        a = (a0 + i * 37) & 0xFF
        b = (i * 91 + 5) & 0xFF
        p = a + b - ((a * b) & 0xFF)
        pa = abs(p - a)
        pb = abs(p - b)
        total += (a if pa <= pb else b) + (i // 3) - (i % 5)
    return total


def window(lo: int, hi: int) -> int:
    out = 0
    for i in range(lo, hi):
        k = i & 0x3F
        out = out * 7 + (k >> 1) - (k << 2) + abs(k - 31)
    return out


def scale(x: int) -> int:
    # `x` is a parameter (any int); `y` is a byte.
    y = x & 0xFF
    return y * 3 + x
