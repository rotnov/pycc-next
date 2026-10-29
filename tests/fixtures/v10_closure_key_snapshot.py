# v0.10: `sorted` keys a copy of its input, so a nested function key that
# changes the list (`xs[1] = 0`, `xs.append`) does not change what is
# sorted; the list itself is changed as the key did.


def mutating(xs: list[int]) -> list[int]:
    def key(v: int) -> int:
        xs[1] = 0
        return v

    return sorted(xs, key=key)


def growing(xs: list[int]) -> list[int]:
    def key(v: int) -> int:
        xs.append(v)
        return -v

    return sorted(xs, key=key, reverse=True)


a = [2, 1]
print(mutating(a), a)
b = [1, 3, 2]
print(growing(b), b)
