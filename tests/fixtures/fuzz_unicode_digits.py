# Found by the differential fuzzer (fuzz/): `int()` and `float()` read any
# Unicode decimal digit (Arabic-Indic, Devanagari, fullwidth, ...), as
# CPython does, and still refuse other non-ASCII characters.
for text in ["١", "١٢٣", " ٤٥ ", "-۷", "１２", "९९", " 12 ", "1_٢"]:
    print(repr(text), int(text))
for text in ["١.٥", "-٣e٢", "１.25"]:
    print(repr(text), float(text))
for text in ["½", "²", "Ⅻ", "١a", "x"]:
    try:
        print(int(text))
    except ValueError as e:
        print("ValueError", e)
try:
    print(float("١,٥"))
except ValueError as e:
    print("ValueError", e)
