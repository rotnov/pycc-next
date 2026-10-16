# Found by the differential fuzzer (fuzz/): a set display of three or more
# constants iterates in the order of CPython's folded frozenset constant,
# which can differ from adding the same elements one by one.
print({-9, 6, 2, 7}, set([-9, 6, 2, 7]))
s: set[int] = {1, 9, 17, 25, 33, 41}
print(s, sorted(s))
for x in {-9, 6, 2, 7}:
    print(x, end=" ")
print()
t: set[str] = {"a", "b", "c", "d"}
print(len(t), {-1, -2, 7}, {(1, 2), (9, -3), (17, 0)}, {0.5, -9.0, 7.0})
n = 6
print({-9, n, 2, 7}, {-9, 7})
