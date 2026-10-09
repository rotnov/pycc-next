import sys
input = sys.stdin.readline

def make_tree(n):
    tree = [0] * (n + 1)
    return tree

def get_sum(i):
    s = 0
    while i > 0:
        s += tree[i]
        i -= i & -i
    return s

def get_sum_segment(s, t):
    ans = get_sum(t) - get_sum(s - 1)
    return ans

def add(i, x):
    while i <= n:
        tree[i] += x
        i += i & -i

t = int(input())
for _ in range(t):
    n = int(input())
    p = list(map(int, input().split()))
    tree = make_tree(n + 5)
    s = set(p)
    s = list(s)
    s.sort()
    d = dict()
    for i in range(len(s)):
        d[s[i]] = i + 1
    cnt = [0] * (n + 1)
    ans = 0
    c0 = 0
    for i in p:
        di = d[i]
        ci = cnt[di]
        s0 = get_sum(di - 1)
        ans += min(s0, c0 - ci - s0)
        add(di, 1)
        c0 += 1
        cnt[di] += 1
    print(ans)
