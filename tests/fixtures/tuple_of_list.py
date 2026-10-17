def _pairs(lines):
    return [tuple(map(int, line.split())) for line in lines]


def main() -> None:
    xs = [3, 1, 2]
    t = tuple(xs)
    xs.append(9)
    print(t, len(t), t[0], t[-1], t[1:], xs)
    print(tuple([5]), tuple(xs[:0]), tuple("abc"), tuple([1.5, 2.0]), tuple(["x", "y"]))
    qs = _pairs(["1 2", "3 4", "5 6"])
    for u, v in qs:
        print(u + v, end=" ")
    print()
    print(qs, qs[0] == (1, 2), tuple([1, 2]) in qs, sorted(qs, reverse=True), max(qs))
    seen = {tuple(sorted([2, 1])), tuple([1, 2]), tuple([3])}
    print(len(seen), tuple(["a", "b"]) + ("c",), tuple([7, 8]) * 2)
    d = {tuple([1, 2]): "x"}
    print(d[tuple([1, 2])], hash(tuple([1, 2])) == hash((1, 2)), 2 in tuple(xs))
    nested = tuple([tuple([1]), tuple([2, 3])])
    print(nested, tuple([10 ** 20, -(10 ** 20)]), tuple(sorted(xs)))
    total = 0
    for x in tuple(xs):
        total += x
    print(total, len(tuple(xs)), tuple(xs)[::-1])
    try:
        a, b = tuple(map(int, "1 2 3".split()))
        print(a, b)
    except ValueError as exc:
        print("ValueError", exc)
    try:
        print(tuple(xs)[10])
    except IndexError as exc:
        print("IndexError", exc)


main()
