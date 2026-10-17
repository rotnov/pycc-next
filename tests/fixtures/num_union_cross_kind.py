"""Numbers of different kinds compared inside containers, and a `float` key
looking up an `int`-keyed dict: CPython compares and hashes numbers by
value, so `(1, 2.5) == (1.0, 2.5)` and `{1: "a"}[1.0]` hold."""


def compare(x: float, n: int) -> None:
    print((1, 2.5) == (1.0, 2.5), (1, 2) != (1.0, 2.0), (n, x) == (2, 0.5))
    print([1, 2] == [1.0, 2.0], [1, 2] < [1.0, 2.5], [n] > [x], [1.5] <= [2])
    print((1, 2) < (1, 2, 0), (1, 2, 0) > (1, 2), (1, 2) == (1, 2, 0), (1,) < (1.5, 0))
    print((1, 2.5) < (1.0, 3), (True, 2) == (1, 2.0), (2 ** 70, 1) > (1e21, 5))
    pairs = [(1, 2.5), (0, 3.5)]
    print(pairs == [(1.0, 2.5), (0.0, 3.5)], max(pairs), (n, x) in [(2, 0.5)])


def lookup(x: float, n: int) -> None:
    d = {1: "a", 2: "b", 2 ** 70: "big"}
    print(d[1.0], d[float(n % 2 + 1)], d.get(2.0, "none"), d.get(1.5, "none"), d[2.0 ** 70])
    y = 1
    if n > 5:
        y = 2.5
    print(d[y], d.get(y, "?"))
    for key in [x, 3.0, float("nan"), float("inf"), -0.0]:
        try:
            print(d[key])
        except KeyError as e:
            print("KeyError", e)
    counts = {0: 10}
    print(counts.get(-0.0, 0), counts[0.0])


compare(0.5, 2)
compare(1.5, 1)
lookup(2.5, 2)
try:
    lookup(1.0, 9)
except KeyError as e:
    print("KeyError", e)
