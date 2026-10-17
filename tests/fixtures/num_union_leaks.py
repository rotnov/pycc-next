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
        # A boxed float is refused by a container (`OverflowError`).
        try:
            kept.append(big)
        except OverflowError:
            acc = acc + 0
    return acc


def echo(x: int | float) -> int | float:
    return x
