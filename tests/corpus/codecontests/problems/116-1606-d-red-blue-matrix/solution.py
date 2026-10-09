import sys

# sys.setrecursionlimit(200005)
int1 = lambda x: int(x)-1
p2D = lambda x: print(*x, sep="\n")
def II(): return int(sys.stdin.readline())
def LI(): return list(map(int, sys.stdin.readline().split()))
def LLI(rows_number): return [LI() for _ in range(rows_number)]
def LI1(): return list(map(int1, sys.stdin.readline().split()))
def LLI1(rows_number): return [LI1() for _ in range(rows_number)]
def SI(): return sys.stdin.readline().rstrip()
# inf = 18446744073709551615
inf = 4294967295
# md = 10**9+7
md = 998244353

def accmin(aa):
    h = len(aa)
    w = len(aa[0])
    res = [[inf]*w for _ in range(h)]
    res[0][0] = aa[0][0]
    for i in range(1, h):
        res[i][0] = min(res[i-1][0], aa[i][0])
    for j in range(1, w):
        res[0][j] = min(res[0][j-1], aa[0][j])
    for i in range(1, h):
        for j in range(1, w):
            res[i][j] = min(res[i-1][j], res[i][j-1], aa[i][j])
    return res

def accmax(aa):
    h = len(aa)
    w = len(aa[0])
    res = [[-1]*w for _ in range(h)]
    res[0][0] = aa[0][0]
    for i in range(1, h):
        res[i][0] = max(res[i-1][0], aa[i][0])
    for j in range(1, w):
        res[0][j] = max(res[0][j-1], aa[0][j])
    for i in range(1, h):
        for j in range(1, w):
            res[i][j] = max(res[i-1][j], res[i][j-1], aa[i][j])
    return res

def solve():
    h, w = LI()
    aa = LLI(h)
    for i in range(h):
        aa[i].append(i)

    aa.sort(key=lambda x: x[0])
    ii = []
    for i in range(h):
        ii.append(aa[i].pop())
    # p2D(aa)
    # print()
    # print(ii)

    lu = accmax(aa)

    aa.reverse()
    ld = accmin(aa)
    ld.reverse()

    for i in range(h): aa[i].reverse()
    rd = accmax(aa)
    rd.reverse()
    for i in range(h): rd[i].reverse()

    aa.reverse()
    ru = accmin(aa)
    for i in range(h): ru[i].reverse()
    ans = ["R"]*h
    for i in range(h-1):
        for j in range(w-1):
            if lu[i][j] < ld[i+1][j] and ru[i][j+1] > rd[i+1][j+1]:
                for k in ii[:i+1]: ans[k] = "B"
                print("YES")
                print("".join(ans), j+1)
                return

    print("NO")

for testcase in range(II()):
    solve()
