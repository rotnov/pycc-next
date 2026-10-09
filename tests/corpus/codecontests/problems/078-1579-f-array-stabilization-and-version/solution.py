import sys
sys.setrecursionlimit(100000)

def _r(): return sys.stdin.buffer.readline()
def rs(): return _r().decode('ascii').strip()
def rn(): return int(_r())
def rnt(): return map(int, _r().split())
def rnl(): return list(rnt())

import collections

def solve(n, d, a):
    sol, q = 0, collections.deque([(i, 0) for i, x in enumerate(a) if x == 0])
    while q:
        i, s = q.popleft()
        ii = (i + d) % n
        if a[ii] == 1:
            a[ii] = 0
            q.append((ii, s+1))
        sol = max(sol, s)
    return sol if sum(a) == 0 else -1

for _ in range(rn()):
    n, d = rnt()
    a = rnl()
    print(solve(n, d, a))
