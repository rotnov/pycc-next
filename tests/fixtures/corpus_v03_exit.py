# The builtins `exit`/`quit` are `sys.exit`: `finally` runs, `except
# Exception` lets `SystemExit` through, the status is the argument.


def check(n):
    if n > 2:
        exit(n)
    return n * 10


try:
    print(check(1))
    try:
        check(5)
    except Exception:
        print("not reached")
finally:
    print("finally runs")
quit(0)
