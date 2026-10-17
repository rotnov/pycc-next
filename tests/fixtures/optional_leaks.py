"""Leak probe for `T | None` values (v0.6 accept 5), called in a loop by
`tests/optional_values.rs`."""


def _first(words, n):
    for w in words:
        if len(w) > n:
            return w
    return None


def churn(n: int) -> int:
    acc = 0
    for i in range(n):
        parents = [None] * n
        for j in range(1, n):
            parents[j] = j - 1
        parents[0] = 10 ** 30 + i
        parents[0] = None
        acc += len([p for p in parents if p is None]) + (1 if parents[1] == 0 else 0)
        words = ["w" + str(i), None, "x" * (i + 1)]
        words[1] = "y" + str(i)
        words[2] = None
        copy = words[:] + words
        acc += len(copy) + (1 if words == copy[:3] else 0)
        label = _first([str(k) * 3 for k in range(i + 2)], 2)
        if label is not None:
            acc += len(label + "!")
        table = {1: "a" + str(i), 2: None}
        table[2] = "b" + str(i)
        acc += len(str(table)) + len(str(words))
        curr = n - 1
        while curr is not None:
            curr = parents[curr]
            acc += 1
    return acc
