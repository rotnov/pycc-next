# #575 (Part 2 of #123): `str * int` and `int * str` repetition.
#
# Negative counts are written as `0 - 2` rather than as a `-2` literal:
# this fixture predates #602's literal-sign fold. The `BinOp::Sub` form is
# still an equally valid way to reach the runtime's non-positive-count rule
# (empty string, matching CPython).

print("ab" * 3)
print(3 * "ab")

print("ab" * 1)
print(1 * "ab")

print("ab" * 0)
print(0 * "ab")

print("ab" * True)
print(True * "ab")
print("ab" * False)
print(False * "ab")

count = 4
print("xy" * count)
print(count * "xy")

zero = 0
print("xy" * zero)

negative = 0 - 2
print("xy" * negative)
print(negative * "xy")

# Crosses the 22-byte inline payload threshold (D-059) so the heap payload
# branch of `pycc_rt_str_repeat` runs in a differential test too.
print("0123456789" * 5)

# Repetition composes with concatenation and with f-strings.
print("ab" * 2 + "!")
doubled = "ab" * 2
print(f"[{doubled}]")


def banner(word: str, width: int) -> str:
    return word * width


print(banner("-", 7))
print(banner("-", 0))
