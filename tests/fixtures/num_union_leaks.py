"""Leak probe for `int | float` (v0.6 accept 3): every runtime path an
`int | float` value takes, with floats too large to be held inline (a
boxed float) and big ints, called in a loop by `tests/num_union.rs`."""

import math


def churn(n: int, x: int | float) -> int | float:
    acc: int | float = 0
    for i in range(n):
        big = x * 1e90
        acc = acc + big / (i + 1)
        acc = acc - big / (i + 1)
        huge: int | float = 10 ** 30 + i
        huge = huge * x
        acc = acc + huge / 1e300
        acc = max(acc, big) if big < 0 else min(acc, abs(big))
        text = f"{big} {big!r} {big:.3e} {huge}" + str(abs(huge)) + format(big, "g")
        acc = acc + len(text) * 0 + hash(big) * 0 + round(big / 1e80) * 0
        acc = acc + math.floor(big / 1e85) * 0 + int(-big / 1e88) * 0
        if big > huge and not (big == huge):
            acc = acc + 0
        acc = acc + float(huge) * 0
        try:
            acc = acc + big / (huge - huge)
        except ZeroDivisionError:
            acc = acc + 0
        try:
            acc = acc + big ** big
        except OverflowError:
            acc = acc + 0
        kept = [i, 1, 2.5]
        acc = acc + sum(kept) * 0 + kept.index(1) * 0
        # A container owns the boxed floats (and big ints) it holds.
        kept.append(big)
        kept.append(huge)
        kept[0] = big * 2
        kept[1] += big
        kept.sort()
        copy = kept[1:] + kept * 2
        acc = acc + max(kept) * 0 + kept.pop() * 0 + len(copy) * 0
        table = {1: big, 2: huge}
        table[3] = big / 2
        table[1] = huge
        keys = {big: 1, huge: 2}
        keys[big] = 3
        seen = {big, huge, big * 2}
        seen.add(big)
        seen.discard(huge)
        acc = acc + table.pop(2) * 0 + keys[big] * 0 + len(seen) * 0
        # An `int | float` or `float` key looks up an `int`-keyed dict.
        ids = {1: big, 2: huge}
        acc = acc + ids[x * 0 + 1.0] * 0 + ids.get(2.0, big) * 0 + ids.get(big, huge) * 0
        try:
            acc = acc + ids[big]
        except KeyError:
            acc = acc + 0
        for value in kept:
            acc = acc + value * 0
        for v in table.values():
            acc = acc + v * 0
    return acc


def echo(x: int | float) -> int | float:
    return x


def num(x: int | float) -> int | float:
    return x


def lookups(n: int, x: int | float) -> int:
    """`float` needles and keys for `int | float` containers: each lookup
    borrows a word converted from the `float`, and a store owns the boxed
    float it converted."""
    xs: list[int | float] = [1, 2.5]
    d: dict[int | float, int] = {}
    d[1] = 10
    s = {num(4), num(0.5)}
    w: dict[str, int | float] = {}
    hits = 0
    for i in range(n):
        big = 1e100 * (i + 1)
        for needle in (big, -big, 1e-100, 2.5):
            hits += int(needle in xs) + int(needle not in d) + int(needle in s)
            hits += d.get(needle, 0)
        hits += int(x in xs) + int(x in d) + int(x in s) + d.get(x, 0)
        try:
            hits += d[big]
        except KeyError:
            hits += 1
        d[big] = 5
        hits += int(w.get("k", 1e300) > 0)
        w["k"] = big
        s.add(1e-300)
    return hits
