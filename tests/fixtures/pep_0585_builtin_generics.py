def _make(n: int) -> list[int]:
    return [n, n * 2, n * 3]


def _demo() -> None:
    x = [10, 20, 30]
    x.append(40)
    print(len(x))
    print(x[0])
    print(x[3])
    for v in x:
        print(v)
    ys: list[int] = _make(5)
    print(len(ys))
    for y in ys:
        print(y)
    print(_make(1)[2])


_demo()
