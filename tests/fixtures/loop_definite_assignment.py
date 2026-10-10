def first_div(n: int) -> int:
    for d in range(2, n):
        if n % d == 0:
            break
    else:
        return -1
    return d


def while_else_bound(limit: int) -> int:
    i: int = 0
    while i < limit:
        sq = i * i
        if sq > 50:
            break
        i += 1
    else:
        return -1
    return sq


def nested(rows: int, cols: int) -> str:
    for r in range(rows):
        for c in range(cols):
            if (r + 1) * (c + 1) == 6:
                break
        else:
            continue
        break
    else:
        return "none"
    return f"{r},{c}"


def checked(v: int) -> int:
    if v < 0:
        raise ValueError("negative")
    return v * 2


def _sum_checked(xs: list[int]) -> int:
    total: int = 0
    for v in xs:
        try:
            r = checked(v)
        except ValueError:
            continue
        total += r
    return total


def _find(xs: list[int], target: int) -> str:
    for x in xs:
        if x == target:
            msg = f"found {x}"
            break
    else:
        msg = "missing"
    return msg


def until_big(n: int) -> int:
    k: int = 1
    while True:
        v = k * n
        if v > 100:
            break
        k += 1
    return v


def odd_squares(n: int) -> int:
    total: int = 0
    for i in range(n):
        if i % 2 == 1:
            sq = i * i
        else:
            continue
        total += sq
    return total


def pick(c: bool) -> int:
    if c:
        x = 1
    else:
        return 0
    return x


def drop_or_keep(c: bool) -> int:
    x = 5
    if c:
        del x
        return -1
    return x


def _must_find(xs: list[int], t: int) -> int:
    for i in range(len(xs)):
        if xs[i] == t:
            break
    else:
        raise ValueError(f"{t} not found")
    return i


def _none_of() -> list[int]:
    xs: list[int] = []
    return xs


empty: list[int] = _none_of()
print(first_div(91), first_div(97), first_div(2), first_div(-5))
print(while_else_bound(100), while_else_bound(3), while_else_bound(0))
print(nested(3, 3), nested(1, 4), nested(0, 0), nested(6, 1))
print(_sum_checked([1, -4, 20, 0, -3]), _sum_checked(empty))
print(_find([1, 2, 3], 2), _find([1, 2, 3], 9), _find(empty, 0))
print(until_big(7), until_big(200), until_big(-1 + 2))
print(odd_squares(10), odd_squares(0))
print(pick(True), pick(False), drop_or_keep(True), drop_or_keep(False))
print(_must_find([5, 6, 7], 7))
try:
    print(_must_find([5, 6, 7], 8))
except ValueError as e:
    print("ValueError:", e)

for q in range(10, 20):
    if q % 7 == 0:
        break
else:
    q = -1
print(q)

w: int = 0
while w < 3:
    w += 1
    last = w * 10
    if last == 20:
        break
else:
    last = -1
print(w, last)
