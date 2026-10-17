def replace_rows(n: int) -> int:
    rows = [(0, n, 1), (1, 2, 3)]
    rows[0] = (n % 5, 7, 1)
    rows[1] = rows[0]
    total = 0
    for a, b, c in rows:
        total += a + b + c
    return total


def ranges(n: int) -> int:
    r = list(map(range, (n, n + 1)))
    r[0] = range(1, n)
    total = 0
    for i in r[0]:
        for j in r[1]:
            total += i * j
    return total


print(replace_rows(3), replace_rows(12), ranges(4), ranges(0))
