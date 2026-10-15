"""`while s[i] in chars: i += 1` (and `not in`) runs as one scan of the
text. Where the loop stops, the index it leaves and the `IndexError` of
reading past either end are CPython's; every case prints what CPython
prints."""

WS = frozenset(" \t")
ACCENTS = frozenset("éè")


def skip(src: str, pos: int, chars: frozenset[str]) -> int:
    try:
        while src[pos] in chars:
            pos += 1
    except IndexError:
        pass
    return pos


def until(src: str, pos: int, stops: set[str]) -> str:
    try:
        while src[pos] not in stops:
            pos += 1
    except IndexError as e:
        return f"IndexError {e} at {pos}"
    return f"stop at {pos}"


def bare(src: str, pos: int) -> int:
    while src[pos] in WS:
        pos += 1
    return pos


def main() -> None:
    print(skip("  \tx", 0, WS), skip("   ", 0, WS), skip("ab", 5, WS), skip("", 0, WS))
    print(skip("a  ", -2, WS), skip(" a", -2, WS), skip("  ", -2, WS))
    print(skip("éèx", 0, ACCENTS), skip("xé ", 1, ACCENTS | WS))
    print(skip("ab", 10**20, WS), skip("ab", -(10**20), WS))
    print(until("abc;d", 0, {";", ","}), until("abc", 0, {";"}), until("éé;", 0, {";"}))
    print(until("abc", -1, {"x"}), until("ab", 10**20, {"x"}))
    for pos in [0, 2, 4]:
        try:
            print(bare("  x ", pos))
        except IndexError as e:
            print("IndexError", e)


main()
