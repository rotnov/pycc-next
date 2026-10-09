import sys
readline = sys.stdin.readline
write = sys.stdout.write

def solve():
    N = int(readline())
    ha, aa, da, sa = map(int, readline().split())
    ans = 0
    S = []
    for i in range(N):
        hi, ai, di, si = map(int, readline().split())
        m0 = max(ai - da, 0)
        if si > sa:
            ans += m0
        m1 = max(aa - di, 0)
        if m1 == 0 and m0 > 0:
            write("-1\n")
            return
        if m0 > 0:
            k = (hi + m1 - 1) // m1
            S.append((k, m0))
    S.sort(key = lambda x: x[0]/x[1])
    cur = 0
    for k, d in S:
        ans += (cur+k-1)*d
        cur += k
    if ans < ha:
        write("%d\n" % ans)
    else:
        write("-1\n")
solve()
