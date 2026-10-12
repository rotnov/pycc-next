import sys
import time

import numpy as np
from numpy.typing import NDArray


def relax(src: NDArray[np.float64], dst: NDArray[np.float64]) -> None:
    n, m = src.shape
    for i in range(1, n - 1):
        for j in range(1, m - 1):
            dst[i, j] = 0.25 * (src[i - 1, j] + src[i + 1, j] + src[i, j - 1] + src[i, j + 1])


def checksum(a: NDArray[np.float64]) -> float:
    n, m = a.shape
    total = 0.0
    for i in range(n):
        for j in range(m):
            total += a[i, j] * (i + 1) / (j + 1)
    return total


def main() -> None:
    n = 200
    m = 300
    a = np.zeros((n, m))
    for j in range(m):
        a[0, j] = 100.0
    for i in range(n):
        a[i, 0] = 50.0
    b = a.copy()
    start = float(time.perf_counter())
    for _ in range(60):
        relax(a, b)
        relax(b, a)
    total = checksum(a)
    stop = float(time.perf_counter())
    elapsed = stop - start
    print(total)
    sys.stderr.write("kernel_seconds=" + str(elapsed) + "\n")


main()
