# `tuple[str, ...]` across the `--ext` boundary: parameters arrive as
# native tuples, results leave as CPython tuples, and a tuple that crossed
# in crosses back as the same object.
type Key = tuple[str, ...]


def parse_key(text: str) -> Key:
    key: Key = ()
    for part in text.split("."):
        key += (part,)
    return key


def parent(key: Key) -> Key:
    return key[:-1]


def same(key: Key) -> Key:
    return key


def stem(key: Key) -> str:
    return key[-1]


def join(a: Key, b: Key) -> Key:
    return a + b


def count(key: Key, part: str) -> int:
    n = 0
    for p in key:
        if p == part:
            n += 1
    return n


def described(key: Key) -> tuple[int, Key, str]:
    return len(key), key, f"{key}"


class Path:
    def __init__(self, key: Key) -> None:
        self.key = key

    def child(self, part: str) -> Key:
        return self.key + (part,)

    def get(self) -> Key:
        return self.key
