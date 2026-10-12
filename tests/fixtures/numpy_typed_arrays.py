import numpy as np
from numpy.typing import NDArray


def total(a: NDArray[np.float64]) -> float:
    s = 0.0
    for i in range(len(a)):
        s += a[i]
    return s


def fill(g: NDArray[np.float64]) -> None:
    n, m = g.shape
    for i in range(n):
        for j in range(m):
            g[i, j] = i * 10.0 + j


def _row_sums(g: NDArray[np.float64]) -> list[float]:
    out: list[float] = []
    for i in range(g.shape[0]):
        s = 0.0
        for j in range(g.shape[1]):
            s += g[i][j]
        out.append(s)
    return out


def prefix(xs: NDArray[np.int64]) -> NDArray[np.int64]:
    out = np.zeros(len(xs), dtype=np.int64)
    acc = 0
    for i in range(len(xs)):
        acc += xs[i]
        out[i] = acc
    return out


def count_true(mask: NDArray[np.bool_]) -> int:
    k = 0
    for flag in mask:
        if flag:
            k += 1
    return k


def scale(g: NDArray[np.float64], factor: float) -> None:
    n, m = g.shape
    for i in range(n):
        for j in range(m):
            g[i][j] *= factor


def first_column(g: NDArray[np.float64]) -> float:
    # `g` is two-dimensional only through its rows.
    s = 0.0
    for row in g:
        s += row[0]
    top = g[0]
    return s + top[1]


class Grid:
    cells: NDArray[np.float64]

    def __init__(self, n: int, m: int) -> None:
        self.cells = np.zeros((n, m))

    def bump(self, i: int, j: int) -> None:
        self.cells[i, j] += 1.0

    def at(self, i: int, j: int) -> float:
        return self.cells[i, j]


def index_errors() -> None:
    a = np.arange(5)
    g = np.ones((2, 3))
    for k in [5, -6, 4, -5]:
        try:
            print("a", k, a[k])
        except IndexError as e:
            print("IndexError:", e)
    for p, q in [(2, 0), (0, 3), (-3, 0), (0, -4), (1, -3)]:
        try:
            print("g", p, q, g[p, q])
        except IndexError as e:
            print("IndexError:", e)
    try:
        print(g[1][3])
    except IndexError as e:
        print("IndexError:", e)
    try:
        g[2, 0] = 1.0
    except IndexError as e:
        print("IndexError:", e)


def read_only() -> None:
    ro = np.zeros(3)
    ro.flags.writeable = False
    try:
        fill(np.zeros(3).reshape(1, 3))
        q: NDArray[np.float64] = ro
        q[0] = 1.0
    except ValueError as e:
        print("ValueError:", e)


def _accumulate(xs: NDArray[np.int64], ys: NDArray[np.int64]) -> None:
    """`x += a[i]` and friends, inside and beyond the inline int range."""
    up = 0
    down = 0
    for i in range(len(xs) - 1):
        up += xs[i]
        down -= xs[i]
    prod = 1
    for i in range(len(ys)):
        prod *= ys[i]
    near = -5
    near += xs[len(xs) - 1]
    far = 5
    far -= xs[0]
    print(int(up), int(down), int(prod), int(near), int(far))


def _compare_elements(xs: NDArray[np.int64], y: int) -> list[int]:
    """An element compared with an int name or literal, on either side."""
    out: list[int] = []
    for i in range(len(xs)):
        out.append(1 if xs[i] > y else 0)
        out.append(1 if y < xs[i] else 0)
        out.append(1 if xs[i] == y else 0)
        out.append(1 if y != xs[i] else 0)
        out.append(1 if xs[i] <= 3 else 0)
        out.append(1 if -2 >= xs[i] else 0)
    return out


def _running_max(xs: NDArray[np.int64]) -> int:
    """The bench's running maximum, from a seed beyond `int64`."""
    best = -(2**70)
    for i in range(len(xs)):
        if xs[i] > best:
            best = xs[i]
    return int(best)


def main() -> None:
    a = np.zeros(6)
    for i in range(6):
        a[i] = i * 0.5
    a[-1] = 9.25
    print(total(a), a[-1], a[0], a.shape, len(a), a.size, a.ndim)
    g = np.zeros((3, 4))
    fill(g)
    print(g[2, 3], g[1][2], g[-1, -1], g.shape[0], g.shape[1], g.size, g.ndim)
    for s in _row_sums(g):
        print(s)
    print(first_column(g))
    scale(g, 2.0)
    print(g)
    v = np.array([3, 1, 4, 1, 5, 9, 2, 6])
    p = prefix(v)
    print(p, p[7], p.dtype)
    big = np.array([1, -1, 0])
    big[0] = 2**62
    big[1] = -(2**62) - 5
    big[2] = 2**62 + (2**62 - 1)
    big[2] -= 2**62
    print(big[0], big[1], big[2], big[2] - 1, big[2] // 3)
    b = np.zeros(3, dtype=np.int64)
    b[0] = 2**62
    b[1] = 2**62 + (2**62 - 1)
    b[2] = True
    print(b)
    try:
        b[0] = 2**63
    except OverflowError as e:
        print("OverflowError:", e)
    mask = np.array([True, False, True, True])
    print(count_true(mask), mask[1], mask[-1])
    mask[1] = True
    print(count_true(mask))
    f = np.full((2, 2), 7.5)
    c = f.copy()
    c[0, 0] = 0.0
    print(f[0, 0], c[0, 0], np.zeros_like(c)[1, 1])
    r = np.arange(3, 9)
    s = 0
    for x in r:
        s += x
    print(s, r[2])
    grid = Grid(2, 3)
    grid.bump(1, 2)
    grid.bump(1, 2)
    print(grid.at(1, 2), grid.cells.sum())
    index_errors()
    read_only()
    edge = np.array([2**62 - 1, 2**62 - 1, 2**63 - 1, -(2**63), 3, -2], dtype=np.int64)
    wide = np.array([2**62 + 5, 2**61, -(2**62), -7, 2**63 - 1], dtype=np.int64)
    _accumulate(wide, np.array([2**31, 2**31 + 1, -1], dtype=np.int64))
    _accumulate(np.array([1, -3, 2], dtype=np.int64), np.array([4, -1], dtype=np.int64))
    print(_compare_elements(edge, 3), _compare_elements(edge, 2**64))
    print(_compare_elements(np.array([4, -2, 3], dtype=np.int64), -(2**65)))
    print(_running_max(edge), _running_max(np.array([-5, -9], dtype=np.int64)))


main()
