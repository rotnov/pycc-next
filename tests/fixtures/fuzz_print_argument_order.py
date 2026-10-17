# Found by the differential fuzzer (fuzz/): `print` converted a dict or set
# argument to text as soon as it was evaluated, so a later argument that
# mutates it (`d.popitem()`) printed the earlier state. CPython evaluates
# every argument first.
d: dict[str, int] = {"a": 1, "b": 2, "c": 3}
print(d, d.popitem(), d)
s: set[int] = {1, 2}
print(s, s.pop(), s)
xs: list[int] = [1, 5]
print(xs, xs.pop(), xs)
for i in range(2):
    print({"k": i}, {i, i + 1}, {str(i): [i]})
print(f"{d}", d.pop("a"), d)
