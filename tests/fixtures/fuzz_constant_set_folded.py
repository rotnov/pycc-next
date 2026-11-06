# Found in review of the differential fuzzer's frozenset fix (fuzz/): a set
# display whose elements the compiler folds (`1 + 0`) is no literal the build
# can read back, so its module must not run the bytecode `marshal` reorders.
# A selected function makes the module run as bytecode.
def selected(n: int) -> int:
    return n + 1


def folded():
    a = {1 + 0, 8, 0}
    b = lambda: {2 * 4, 1, 0}

    def inner():
        return {0, 8 - 0, 1}, (lambda: {(1 + 0, 8), (8, 1), (0, 0)})()

    return a, b(), inner()


print(folded(), selected(1))
