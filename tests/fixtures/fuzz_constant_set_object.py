# Found by the differential fuzzer (fuzz/): a set display of constants built
# as a CPython object (`--non-strict`: `str(...)` of it) iterates in the order
# of CPython's folded frozenset constant.
print(str({8, 6, 0, 7, -9}), {-9, 6, 2, 7} | set({}), repr({1, 9, 17, 25, 33, 41}))
