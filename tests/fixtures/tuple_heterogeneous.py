t = (1, True, 2.5)
print(t[0])
print(t[1])
print(t[2])


def second() -> int:
    u = (10, 20, 30)
    return u[1]


def pick_float() -> float:
    v = (1.5, 2.5)
    return v[0]


print(second())
print(pick_float())
print((7, 8)[1])
