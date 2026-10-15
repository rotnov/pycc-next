# A chained comparison over CPython objects is the conjunction of its links,
# each an object comparison, with the value CPython gives it.
import json

o = json.loads("[1, 5, null, [2, 3]]")
a = o[0]
b = o[1]
n = o[2]
xs = o[3]
print(0 < a < b < 10, a == 1 == a, n == None == n, 1 < b > 9, a < b < 3)
print(a == 1 < b, xs != a == 1, 0 <= a < 2 <= b)
if n == n == None:
    print("both none")
total = 0
for k in o[3]:
    if 0 < k < b <= 5:
        total += k
print(total)
