import sys

input = sys.stdin.buffer.readline
t = int(input())

for _ in range(t):
    res = 0
    n = int(input())
    A = list(map(int, input().split()))
    flag = 0
    a,b,c=A[0],A[1],A[-1]
    if c>=a+b:
        print(1,2,n)
        flag=1
    if not flag:
        a,b,c=A[0],A[-2],A[-1]
        if c-b>=a:
            print(1,n-2,n-1)
            flag=1
    if not flag:
        print(-1)
