"""Leak probe for `tuple(xs)` (v0.6 accept 5), called in a loop by
`tests/tuple_of_list.rs`."""


def churn(n: int) -> int:
    acc = 0
    for i in range(n):
        xs = [i, i + 1, 10 ** 30]
        t = tuple(xs)
        names = tuple(["a" + str(i), "b"])
        chars = tuple("xyz" + str(i))
        rows = [tuple(xs), tuple(xs[:2])]
        rows[0] = tuple(xs[1:])
        for a, b in rows:
            acc += a + b
        acc += len(t) + len(names[0]) + len(chars) + len(str(rows))
        acc += hash(t) * 0 + (1 if t in rows else 0)
    return acc
