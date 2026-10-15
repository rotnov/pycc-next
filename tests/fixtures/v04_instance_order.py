from dataclasses import dataclass


class Version:
    def __init__(self, major: int, minor: int) -> None:
        self.major = major
        self.minor = minor

    def __lt__(self, other: "Version") -> bool:
        if self.major != other.major:
            return self.major < other.major
        return self.minor < other.minor

    def __repr__(self) -> str:
        return f"v{self.major}.{self.minor}"


class Score:
    """Defines only `__gt__`: `<` falls back to the reflected `__gt__`."""

    def __init__(self, n: int, tag: str) -> None:
        self.n = n
        self.tag = tag

    def __gt__(self, other: "Score") -> bool:
        return self.n > other.n

    def __repr__(self) -> str:
        return f"{self.tag}{self.n}"


@dataclass(order=True, frozen=True)
class Card:
    rank: int
    suit: str


@dataclass(unsafe_hash=True)
class Tag:
    name: str


class Key:
    def __init__(self, k: int) -> None:
        self.k = k

    def __eq__(self, other: "Key") -> bool:
        return self.k % 10 == other.k % 10

    def __hash__(self) -> int:
        return self.k % 10


class Picky:
    def __init__(self, n: int) -> None:
        self.n = n

    def __lt__(self, other: "Picky") -> bool:
        if self.n == other.n:
            raise ValueError(f"tie at {self.n}")
        return self.n < other.n


def main() -> None:
    vs = [Version(1, 2), Version(0, 9), Version(1, 0), Version(0, 9)]
    print(sorted(vs), vs)
    print(min(vs), max(vs), min(Version(3, 1), Version(2, 7)), max(Version(3, 1), Version(2, 7), Version(3, 0)))
    vs.sort()
    print(vs)
    # A stable sort keeps equal elements in their order.
    scores = [Score(3, "a"), Score(1, "b"), Score(3, "c"), Score(2, "d"), Score(1, "e")]
    print(sorted(scores), min(scores), max(scores))
    empty: list[Version] = []
    print(sorted(empty))
    empty.sort()
    print(empty)
    # Dataclass ordering, `key=` and `reverse=` still work.
    cards = [Card(10, "h"), Card(2, "s"), Card(10, "c"), Card(7, "d")]
    print(sorted(cards))
    print(sorted(cards, key=lambda c: c.suit), sorted(cards, reverse=True)[0])
    print(min(cards), max(cards))
    # A key that is itself an instance, and `reverse=` without a key.
    print(sorted(scores, reverse=True), sorted(vs, key=lambda v: v, reverse=True))
    pairs = [(Score(2, "x"), "two"), (Score(1, "y"), "one"), (Score(2, "z"), "deux")]
    print(sorted(pairs, key=lambda p: p[0]), min(pairs, key=lambda p: p[0])[1], max(pairs, key=lambda p: p[0])[1])
    scores.sort(reverse=True)
    print(scores)
    vs.sort(key=lambda v: Version(v.minor, v.major))
    print(vs)
    # Set membership through `__hash__`/`__eq__`.
    hand = {Card(10, "h"), Card(2, "s")}
    print(Card(10, "h") in hand, Card(10, "s") in hand, Card(3, "s") not in hand)
    tags = {Tag("x"), Tag("y"), Tag("x")}
    print(len(tags), Tag("y") in tags, Tag("z") in tags)
    keys = {Key(1), Key(22)}
    print(Key(11) in keys, Key(3) in keys, len(keys))
    # A comparison that raises stops the sort with its exception.
    try:
        sorted([Picky(3), Picky(1), Picky(3)])
    except ValueError as e:
        print("ValueError:", e)
    print(min([Picky(5), Picky(2)]).n)
    # `min()` of an empty list raises CPython's ValueError.
    print(max(empty))


main()
