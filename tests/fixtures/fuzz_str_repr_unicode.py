# Found by the differential fuzzer (fuzz/): `repr()` of a `str` escapes every
# character `str.isprintable()` rejects (a no-break space, a soft hyphen, a
# zero-width space, a line separator, a tag character, a private-use or
# unassigned code point), as \xNN, \uNNNN or \UNNNNNNNN.
words: list[str] = ["\t\xa0 x", "soft\xadhyphen", "zero​width", "line sep",
                    "tag\U000e0001", "private", "unassigned͸", "ok é 日 😀 ñ"]
for w in words:
    print(repr(w), [ch for ch in w][-2:], f"{w!r:>30}")
print(words)
d: dict[str, int] = {"\xa0": 1}
print(d)
