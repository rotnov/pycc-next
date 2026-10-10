def check(i: int) -> int:
    if i % 2 == 0:
        raise ValueError("even")
    return i


def try_finally(n: int) -> float:
    t: float = 0.0
    i: int = 0
    while i < n:
        try:
            t += i / 2
        finally:
            t -= 1
        i += 1
    return t


def except_as(n: int) -> int:
    caught: int = 0
    for i in range(n):
        try:
            check(i)
        except ValueError as e:
            if i == 0:
                print("first:", e)
            caught += 1
    return caught


def try_finally_return(n: int) -> int:
    hits: int = 0
    for i in range(n):
        try:
            if i == n - 1:
                return hits
        finally:
            hits += 1
    return -1


n: int = 5000000
print(try_finally(n))
print(except_as(n))
print(try_finally_return(n))

top: int = 0
k: int = 0
while k < n:
    try:
        check(k)
    except ValueError as err:
        top += 1
    finally:
        top += 1
    k += 1
print(top)
