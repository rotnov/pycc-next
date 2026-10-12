# Non-strict mode: a CPython object used as a list or str index or as a
# range() argument is read through __index__, with CPython's TypeError for
# a non-index object in each position.
import json

o = json.loads("2")
f = json.loads("1.5")
xs = [10, 20, 30]
s = "abcd"
print(xs[o], s[o], xs[-o])
for i in range(o):
    print(i)
xs[o] = 5
print(xs, [i * 2 for i in range(o, 4)], list(range(1, 9, o)))
big = json.loads("9" * 30)
print(xs[o:], xs[:o], s[1:o + 1], s[::o], xs[-big:big], s[big:])
try:
    print(xs[f:])
except TypeError as e:
    print(e)

try:
    print(xs[f])
except TypeError as e:
    print(e)
try:
    print(s[f])
except TypeError as e:
    print(e)
try:
    for i in range(f):
        print(i)
except TypeError as e:
    print(e)
try:
    xs[f] = 1
except TypeError as e:
    print(e)
xs[o + 5]
