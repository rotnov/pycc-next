"""Non-strict mode: a name one function rebinds with another type becomes a
CPython object in that function only; another function's local of the same
name keeps its native type."""


def rebinds(n):
    delta = 0
    if n > 2:
        delta = "big"
    return delta


def counts(a, b):
    delta = a - b
    total = 0
    for i in range(delta):
        total += i % 7
    return total


class Walker:
    def __init__(self):
        self.steps = 0

    def label(self, k):
        step = k
        if k > 1:
            step = "many"
        return step

    def walk(self, k):
        step = k * 2
        while step > 0:
            self.steps += step % 3
            step -= 1
        return self.steps


w = Walker()
print(rebinds(1), rebinds(5), counts(10, 3), counts(3, 10))
print(w.label(1), w.label(4), w.walk(5), w.walk(2))
