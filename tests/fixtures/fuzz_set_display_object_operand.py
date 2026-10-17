# Found by the differential fuzzer (fuzz/): in `--non-strict` mode a set
# display used as the operand of an operation on a CPython object crossed
# as a `frozenset`, so `{1} ^ set({})` printed `frozenset({1})`.
x = set({})
print({1} ^ x, {1, 2} | x, type({3} - x).__name__)
