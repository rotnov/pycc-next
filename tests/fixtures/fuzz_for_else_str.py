# Found by the differential fuzzer (fuzz/): a `for ... else` over a `str` (or
# a tuple), whose loop is lowered after binding the list of its items, ran
# its `else:` clause even when the loop ended with `break`.


def scan(s: str, stop: str) -> None:
    for c in s:
        if c == stop:
            print("found", c)
            break
    else:
        print("not found in", s)


scan("abc", "b")
scan("abc", "z")
scan("", "a")
for c in repr(0):
    break
else:
    print("never")
t: tuple[int, int, int] = (1, 2, 3)
for x in t:
    if x == 2:
        break
else:
    print("never either")
for x in t:
    continue
else:
    print("tuple loop finished")
for w in "héllo":
    pass
else:
    print("str loop finished")
