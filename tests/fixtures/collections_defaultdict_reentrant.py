# A defaultdict factory that drops the last reference to the dict it is
# filling: the dict must stay alive until the lookup (and a fused
# `d[k] += n`) has finished.
from collections import defaultdict


def _make() -> list[int]:
    global table
    table = defaultdict(list)
    return [7]


def _zero() -> int:
    global counts
    counts = defaultdict(int)
    return 5


table: defaultdict[str, list[int]] = defaultdict(_make)
print(table["a"])
print(table["b"], len(table))

counts: defaultdict[str, int] = defaultdict(_zero)
counts["a"] += 1
print(dict(counts), len(counts))
