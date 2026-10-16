# Found by the differential fuzzer (fuzz/): a `KeyError` for an `int` (or
# tuple) key escaping an `--ext` module reached CPython holding the key's
# text, so it printed `KeyError: '0'` instead of `KeyError: 0`.
d: dict[int, str] = {4: "x", -2: "y"}
t: dict[tuple[int, str], int] = {(1, "a"): 1}
try:
    print(t[(2, "b")])
except KeyError as e:
    print("caught", e)
try:
    print(d[0])
except KeyError as e:
    print("caught", e)
print({4: "日", 1: "é"}[-7])
