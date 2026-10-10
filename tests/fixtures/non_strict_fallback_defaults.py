# Non-strict: fallback code (the lambdas) calls compiled functions with a
# defaulted parameter left out. A published callable cannot apply defaults
# yet, so these functions run in CPython too.
def scale(x: object, k: object = 2) -> object:
    return x * k


def scale2(x: int, k: int = 2) -> int:
    return x * k


print((lambda: scale(3))())
print((lambda: scale2(3))())
