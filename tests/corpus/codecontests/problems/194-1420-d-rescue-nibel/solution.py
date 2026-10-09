import sys

def input(): return sys.stdin.readline().strip()
def list2d(a, b, c): return [[c for j in range(b)] for i in range(a)]
def list3d(a, b, c, d): return [[[d for k in range(c)] for j in range(b)] for i in range(a)]
def list4d(a, b, c, d, e): return [[[[e for l in range(d)] for k in range(c)] for j in range(b)] for i in range(a)]
def ceil(x, y=1): return int(-(-x // y))
def INT(): return int(input())
def MAP(): return map(int, input().split())
def LIST(N=None): return list(MAP()) if N is None else [INT() for i in range(N)]
def Yes(): print('Yes')
def No(): print('No')
def YES(): print('YES')
def NO(): print('NO')
INF = 10**19
MOD = 998244353
EPS = 10**-10

def compress(S):

    zipped, unzipped = {}, {}
    for i, a in enumerate(sorted(S)):
        zipped[a] = i
        unzipped[i] = a
    return zipped, unzipped

class ModTools:

    def __init__(self, MAX, MOD):

        MAX += 1
        self.MAX = MAX
        self.MOD = MOD
        factorial = [1] * MAX
        factorial[0] = factorial[1] = 1
        for i in range(2, MAX):
            factorial[i] = factorial[i-1] * i % MOD
        inverse = [1] * MAX
        inverse[MAX-1] = pow(factorial[MAX-1], MOD-2, MOD)
        for i in range(MAX-2, -1, -1):
            inverse[i] = inverse[i+1] * (i+1) % MOD
        self.fact = factorial
        self.inv = inverse

    def nCr(self, n, r):

        if n < r: return 0
        r = min(r, n-r)
        numerator = self.fact[n]
        denominator = self.inv[r] * self.inv[n-r] % self.MOD
        return numerator * denominator % self.MOD

N, K = MAP()
LR = []
S = set()
for i in range(N):
    l, r = MAP()
    r += 1
    LR.append((l, r))
    S.add(l)
    S.add(r)

zipped, _ = compress(S)
M = len(zipped)
lcnt = [0] * M
rcnt = [0] * M
for i in range(N):
    LR[i] = (zipped[LR[i][0]], zipped[LR[i][1]])
    lcnt[LR[i][0]] += 1
    rcnt[LR[i][1]] += 1

cur = 0
ans = 0
mt = ModTools(N, MOD)
for i in range(M):
    cur -= rcnt[i]
    while lcnt[i]:
        if cur >= K-1:
            ans += mt.nCr(cur, K-1)
            ans %= MOD
        cur += 1
        lcnt[i] -= 1
print(ans)
