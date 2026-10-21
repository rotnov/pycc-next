def _mk(s, n):
    return [s, n]


def main() -> None:
    # A list display of a `str` and an `int` is a list of a fixed shape.
    b = ["k", 3]
    c = ["k", 4, 2.5]
    print(b, b[0], b[1], len(b), str(b), repr(b), f"{b}")
    print(b == ["k", 4], b != ["k", 3], b < ["k", 4], ["a", 9] <= b, c > ["k", 4, 1.0])
    x, y, z = c
    print(x, y, z)
    # Lists of them sort lexicographically, as CPython's lists do.
    a = []
    for i in range(4):
        p = "x" + str(3 - i % 2)
        a.append([p, i + 1])
    a.sort()
    print(a)
    for name, index in a:
        print(name, index, end=" ")
    print()
    for row in a:
        print(row[1], end=" ")
    print()
    rows = [[str(i), i * 2] for i in range(4)]
    rows.sort(reverse=True)
    print(rows, sorted(rows, key=lambda r: -r[1])[0], max(rows), min(rows))
    print(_mk("z", 9), _mk("z", 9)[0], {"x": b}, (b, 1), [b, b])


main()
