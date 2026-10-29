# v0.10: a `min`/`max` key lambda reading a local the function rebinds
# later is called within the call, so it reads the value the local holds
# then (the pairs' generator is consumed whole inside `min`/`max`).


def scaled(xs: list[int]) -> list[int]:
    n = 2
    low = min(xs, key=lambda x: n * x)
    high = max(xs, key=lambda x: (x - n) * (x - n))
    n += 1
    return [low, high, n]


def ranked(words: list[str]) -> str:
    bonus = 1
    best = max(words, key=lambda w: len(w) * bonus)
    bonus = -1
    worst = max(words, key=lambda w: len(w) * bonus)
    return best + " " + worst


print(scaled([3, -1, 4]))
print(ranked(["aa", "b", "cccc"]))
