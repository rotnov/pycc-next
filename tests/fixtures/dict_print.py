# `print`/`str`/f-string and truthiness of a `dict[str, int]`, CPython's
# repr: insertion order, quoted keys (double quotes when a key holds a single
# quote), large values, the empty dict; `str()` of a list.
d = {}
print(d)
d["a"] = 1
d["it's"] = -5
d["b"] = 2**61
d["a"] = 3
print(d)
print(str(d), f"{d}!")
counts = {"x": 0}
for ch in "xyx":
    counts[ch] = counts.get(ch, 0) + 1
print(counts, len(counts))
xs = [3, 1]
print(str(xs) + "!", str([["a"], ["b"]]), len(str([1.5, 2.0])))
e = {}
if e:
    print("non-empty")
else:
    print("empty")
e["k"] = 0
while e:
    print("truthy", e)
    break
