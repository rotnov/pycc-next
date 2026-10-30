"""Median and spread of repeated timings, shared by the `--json` outputs of
`bench/run.py`, `bench/package_check.py --time` and `bench/pydantic_check.py --time`
(`bench/publish.py` turns them into the CI benchmark workflow's tables).

`summary(xs)` is a dict of seconds: `n`, `median`, `min`, `max`, `q1`, `q3` (the
quartiles by `statistics.quantiles`' inclusive method; with one sample all are that
sample). `short(s, ms)` prints one as "median (min-max)", in milliseconds when `ms`,
else in seconds.
"""
import statistics


def summary(xs: list[float]) -> dict[str, float]:
    if not xs:
        raise ValueError("no samples")
    if len(xs) == 1:
        q1 = q3 = xs[0]
    else:
        q1, _, q3 = statistics.quantiles(xs, n=4, method="inclusive")
    return {"n": len(xs), "median": statistics.median(xs), "min": min(xs),
            "max": max(xs), "q1": q1, "q3": q3, "samples": list(xs)}


def interleaved(runs: int, names: list[str]) -> list[list[str]]:
    """The order of the `runs` rounds: each round runs every name once, rotated by one
    per round, so no side always runs first (on a warm or a cold cache)."""
    return [names[i % len(names):] + names[:i % len(names)] for i in range(runs)]


def short(s: dict[str, float], ms: bool = False) -> str:
    if ms:
        return f"{s['median'] * 1000:.2f} ms ({s['min'] * 1000:.2f}-{s['max'] * 1000:.2f})"
    return f"{s['median']:.3f}s ({s['min']:.3f}-{s['max']:.3f})"
