import sys
input = lambda: sys.stdin.readline().strip()

# sys.stdin = open('input.txt', 'r')
# sys.stdout = open('output.txt', 'w')

mp = {'+': 1, '-': -1}

def solve():
    n,m = map(int, input().split())
    s = input()
    u = [0] + [mp[c]*(1 if i&1 else -1) for i,c in enumerate(s)]
    for i in range(1,n+1):
        u[i] += u[i-1]
    for _ in range(m):
        l,r = map(int, input().split())
        x = u[r] - u[l-1]
        if x==0:
            print(0)
        elif (r-l+1)&1:
            print(1)
        else:
            print(2)
    return

for _ in range(int(input())):
    solve()
