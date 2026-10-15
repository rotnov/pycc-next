# An uncaught call-before-def `NameError` ends the program as CPython does:
# a traceback whose last line is the exception, and exit status 1.


def first() -> int:
    return second(1)


print("before")
print(first())


def second(x: int) -> int:
    return x
