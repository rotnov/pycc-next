# A computed index into a tuple whose elements share one int/bool/float
# type; an index outside [-len, len) raises CPython's IndexError.
dirs = {0: (True, False, True, False), 1: (False, False, True, True)}
t = dirs[1]
for k in range(4):
    print(k, t[k], t[-k - 1])
w = (1.5, 2.5, 3.5)
i = 1
print(w[i], w[i + 1], w[True])


def pick(p, q):
    return p[q]


print(pick((7, 8, 9), 2), pick((7, 8, 9), -3))
j = 4
try:
    print(t[j])
except IndexError as e:
    print("IndexError", e)
print(t[-j - 1])
