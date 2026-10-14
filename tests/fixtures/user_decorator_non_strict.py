from functools import wraps


def logged(f):
    @wraps(f)
    def wrapper(*args):
        print("calling", f.__name__, args)
        return f(*args)

    return wrapper


@logged
def add(a: int, b: int) -> int:
    return a + b


def twice(n: int) -> int:
    return add(n, n)


def main() -> None:
    print(add(1, 2))
    print(twice(5))


main()
