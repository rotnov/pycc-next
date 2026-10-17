# v0.5 corpus round: `float('inf')` in a variable an `int` later replaces,
# and `min`/`max` of a `str`.
import math

out = []
for m in [1, 2, 3]:
    bob = float('inf')
    for v in range(m):
        ans = (v * 7) % 5
        if ans < bob:
            bob = ans
    out.append(bob)
out.append(0)
for e in out:
    print(e)

ans = float('inf')
for g in [9, 4, 6]:
    ans = min(ans, g)
print(ans)

res = float("inf")
for g in range(0):
    res = min(res, g)
if res == float('inf'):
    print(-1, res)
else:
    print(res)

best = -float('inf')
print(best, best < 0, best > -10**30, best == -math.inf)
best = max(best, 3)
print(best)

never = math.inf
if len(out) > 100:
    never = 5
print(never > 10**100, str(never), f"{never}|{never!r}", never + 5, never - 7)
print(never * 2, never * -1, never / 2, 3 / never, -3 / never, abs(-never))
print(hash(never) == hash(float('inf')), not never, float(never), never >= never)
print(min(never, 2), max(never, 2), min(2, never, 1), never != 1.5, 2.5 < never)
lo = float('-inf')
if len(out) > 100:
    lo = 1
print(lo, lo < -10**40, max(lo, -7), lo + never if False else lo - 1)
try:
    print(never / 0)
except ZeroDivisionError as e:
    print("ZeroDivisionError:", e)

s = "4021"
print(min(s), max(s), min("zyx"), max("a"), min(s + "0" * (len(set(s)) < 9)))
try:
    print(min(""))
except ValueError as e:
    print("ValueError:", e)

# A name whose other bindings are floats keeps its float infinity.
f = float('inf')
for v in [2.5, 1.25]:
    f = min(f, v)
print(f, float('inf') / 2, -float('inf'))

# v0.6: the name is an `int | float`, so the float arithmetic a v0.5 `int`
# infinity refused is CPython's.
x = float('inf')
if len('ab') > 5:
    x = 1
print(x // 2, x % 2, x - x, x * 0, x ** 2, [x], (x, 1), {x: 1})
