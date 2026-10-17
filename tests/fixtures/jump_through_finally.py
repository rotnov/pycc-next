"""`break` and `continue` out of a `try` that has a `finally:` clause: the
`finally` block runs first, as in CPython."""


class Stop(Exception):
    pass


def risky(n: int) -> int:
    if n == 3:
        raise ValueError("three")
    if n == 5:
        raise Stop("five")
    return n * 2


def loop(limit: int) -> int:
    out = 0
    for i in range(10):
        try:
            out += risky(i)
            if i == limit:
                return out
        except ValueError:
            out -= 1
            continue
        except Stop:
            break
        finally:
            print("fin", i, out)
        out += 100
    return out


print(loop(1), loop(7))

n = 0
while True:
    n += 1
    try:
        if n % 2 == 0:
            continue
        if n > 6:
            break
        print("odd", n)
    finally:
        print("always", n)
print("done", n)

for i in range(3):
    try:
        try:
            if i == 1:
                continue
        finally:
            print("inner", i)
        for j in range(3):
            if j == 1:
                break
            print("j", j)
        else:
            print("never")
        if i == 2:
            break
    except ValueError:
        pass
    else:
        print("else", i)
    finally:
        print("outer", i)

total = 0
for k in range(4):
    try:
        match k:
            case 0:
                continue
            case 3:
                break
            case _:
                total += k
    finally:
        total += 10
print(total)
