import math as m

print(m.sqrt(16.0))
print(m.pi)


def hyp(a: float, b: float) -> float:
    return m.sqrt(a * a + b * b)


print(hyp(3.0, 4.0))
