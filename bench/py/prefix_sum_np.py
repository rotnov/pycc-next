import sys
import time

import numpy as np
from numpy.typing import NDArray


def fill(xs: NDArray[np.int64], seed: int) -> None:
    x = seed
    for i in range(len(xs)):
        x = (x * 1103515245 + 12345) % 2147483648
        xs[i] = x % 2001 - 1000


def prefix_sums(xs: NDArray[np.int64], out: NDArray[np.int64]) -> None:
    acc = 0
    for i in range(len(xs)):
        acc += xs[i]
        out[i] = acc


def running_max(xs: NDArray[np.int64], out: NDArray[np.int64]) -> None:
    best = xs[0]
    for i in range(len(xs)):
        if xs[i] > best:
            best = xs[i]
        out[i] = best


def main() -> None:
    n = 400000
    xs = np.zeros(n, dtype=np.int64)
    sums = np.zeros(n, dtype=np.int64)
    peaks = np.zeros(n, dtype=np.int64)
    fill(xs, 42)
    start = float(time.perf_counter())
    check = 0
    for r in range(25):
        xs[r] += 1
        prefix_sums(xs, sums)
        running_max(sums, peaks)
        check += sums[n - 1] + peaks[n - 1] + peaks[n // 3]
    stop = float(time.perf_counter())
    elapsed = stop - start
    print(check)
    sys.stderr.write("kernel_seconds=" + str(elapsed) + "\n")


main()
