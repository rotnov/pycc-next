# v0.10: nested functions used as values whose captures are rebound before
# the `def` (a parameter normalized first), and handed to `map`, `filter`
# and `sorted(key=...)`, which call the lifted function directly.


def render(template: str, items: list[str]) -> str:
    template = template.strip()

    def piece(item: str) -> str:
        return template + ":" + item

    return ",".join(list(map(piece, items)))


def keep(prefix: str, items: list[str]) -> list[str]:
    prefix = prefix.lower()
    n = len(prefix)
    n = n + 0

    def starts(item: str) -> bool:
        return item[:n].lower() == prefix

    return list(filter(starts, items))


def order(items: list[str], reverse_digits: bool) -> list[str]:
    weight = 1
    if reverse_digits:
        weight = -1

    def key(item: str) -> int:
        return weight * len(item)

    return sorted(items, key=key)


def returned(base: int) -> int:
    base = base * 2

    def add(x: int) -> int:
        return x + base

    total = 0
    for v in map(add, [1, 2, 3]):
        total += v
    return total


print(render("  T ", ["a", "b", "c"]))
print(render("", []))
print(keep("AB", ["abc", "xab", "ABd", "a"]))
print(order(["ccc", "a", "bb"], False))
print(order(["ccc", "a", "bb"], True))
print(returned(5))
