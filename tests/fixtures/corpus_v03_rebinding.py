# A local rebound from a scalar to an empty list is two variables; `list()`
# and `dict()` are the empty displays; `set.update`/`list.extend` take a
# generator.
import itertools


def func():
    t = "abcab"
    tc = [0] * 26
    for i in range(len(t)):
        temp = ord(t[i])
        tc[temp - 97] += 1
    temp = []
    for m in range(3, 0, -1):
        temp.append(t[m])
    temp = temp[::-1]
    print("".join(temp), tc[:3])
    seen = list()
    counts = dict()
    for ch in t:
        seen.append(ch)
        counts[ch] = counts.get(ch, 0) + 1
    print(seen, counts)


func()
close = set()
for g in [[1, 2, 3], [2, 5]]:
    close.update(itertools.combinations(g, 2))
print(sorted(close), (1, 3) in close)
xs = []
xs.extend(i * i for i in range(4))
print(xs)
