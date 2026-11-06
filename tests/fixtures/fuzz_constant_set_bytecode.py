# Found by the differential fuzzer (fuzz/): a module run as CPython bytecode
# loads its code objects with `marshal`, which writes a frozenset constant's
# elements sorted; each set display of constants must still iterate as a
# compile of this source orders it.
def b1_f0(p0: tuple, p1: tuple) -> dict:
    b1_1: str = {1, 8, 0}
    print(b1_1, sep=',')


def f():
    a = {1, 8, 0}
    print([v for v in a], a, {1, 8}, {1, 8, 0, 3})
    print(9 in {9, 1, 17, 33}, {0, 8, 1}, {2, 10} | {1, 9, 17})

    def g():
        return {33, 1, 17, -3}, {(9, 1), (1, 9), (17, 0)}

    print(g())


b1_f0(0, 1)
f()
print({40, 8, 0, 16}, {-1, -9, 7})
