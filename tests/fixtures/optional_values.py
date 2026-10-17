def _first_long(words: list[str], n: int) -> str | None:
    for w in words:
        if len(w) > n:
            return w
    return None


def _grid(n: int) -> list[list[str]] | None:
    if n == 0:
        return None
    return [["."] * n for _ in range(n)]


def pointer_optionals() -> None:
    to_fill: str | None = None
    for el in ["b", "d", "a"]:
        if to_fill is None:
            to_fill = el
        else:
            to_fill = min(to_fill, el)
    print(to_fill)
    if to_fill is not None:
        print("x" + to_fill)
    w = _first_long(["ab", "abcd"], 3)
    print(w, _first_long(["a"], 3), str(w), f"<{w}>", w == "abcd", w != "abcd")
    print(_first_long([], 0) == "x", _first_long([], 0) != "x")
    g = _grid(2)
    if g:
        for row in g:
            print("".join(row))
    print(_grid(0), _grid(1))
    h = _grid(0)
    if not h:
        print("empty")
    names: list[str] | None = None
    print(names is None, names)
    names = ["q"]
    print(names is None, names)


def optional_lists(n: int) -> None:
    parents = [None] * n
    biggest = [0] * n
    for i in range(1, n):
        parents[i] = i - 1
    biggest[2] = None
    print(parents, biggest, parents[0] is None, parents[1] == 0, parents[2] != 5)
    curr = n - 1
    while curr is not None:
        print(curr, end=" ")
        curr = parents[curr]
    print()
    best = None
    for g in [3, 1, 4]:
        if g > 2:
            best = g
    biggest[0] = best
    print(biggest, best == 4, 4 == best, biggest[2] == None, len([b for b in biggest if b is None]))
    big = [None, 10 ** 30]
    big[0] = -(10 ** 25)
    print(big, big[1] == 10 ** 30, big[0] is not None)
    words = ["a", None, "c"]
    print(words, words[1] is None, words == ["a", None, "c"], len(words))
    joined: list[str] = []
    for w in words:
        if w is not None:
            joined.append(w)
    print("-".join(joined))
    labels = {1: "one", 2: None}
    print(labels, labels[2] is None)
    try:
        print(labels[3])
    except KeyError as exc:
        print("KeyError", exc)


pointer_optionals()
optional_lists(4)
