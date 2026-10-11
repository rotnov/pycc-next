res = []
for t in range(3):
    n = t + 2
    vals = [i * i for i in range(n)]

    def cal(x, y):
        if x > y:
            return -1
        return vals[x] + vals[y] + n

    def rec(k):
        if k == 0:
            return 0
        return k + rec(k - 1)

    res.append(cal(0, n - 1) + rec(n))
print(res)
if len(res) > 2:
    def show(xs):
        return " ".join(str(v) for v in xs)
    print(show(res))
else:
    print("short")
