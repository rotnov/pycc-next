import math, math as m

print(math.sqrt(16.0), m.pi)


def hyp(a: float, b: float) -> float:
    return m.sqrt(a * a + b * b)


print(hyp(3.0, 4.0), math.sqrt(2.25))
