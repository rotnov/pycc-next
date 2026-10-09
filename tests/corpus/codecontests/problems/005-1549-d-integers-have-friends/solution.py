import math
for case in range(int(input())):
    n = int(input())
    a = [int(i) for i in input().split()]
    b = [1] + [abs(a[i]-a[i-1]) for i in range(1, n)]
    cur = {1 : 0}
    ans = 0
    for i in range(1, n):
        g = b[i]
        d = {}
        d[g] = i
        if g != 1:
            tmp = sorted(list(cur.keys()), reverse = True)
            for j in tmp:
                g = math.gcd(g, j)
                if g not in d:
                    d[g] = cur[j]
                if g == 1:
                    ans = max(ans, i - cur[j])
                    break
        cur = d
    print(ans + 1)
