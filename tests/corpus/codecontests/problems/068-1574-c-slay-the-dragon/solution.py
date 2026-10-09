from sys import stdin
from bisect import bisect_left
input = stdin.readline
n=int(input())
arr=list(map(int,input().split()))
arr.sort()
m=int(input())
s=sum(arr)
for i in range(m):
    x,y=map(int,input().split())
    if s<=y or s>y:
        # pos=bisect_right(arr,x)
        ans=float('inf')
        # # print(pos)
        # if pos<n:
        #     g=max(0,y-(s-arr[pos]))
        #     ans=min(ans,g)
        pos=bisect_left(arr,x)
        if pos<n:
            g=max(0,y-(s-arr[pos]))+max(0,x-arr[pos])
            ans=min(ans,g)
        pos=pos-1
        if pos>=0:
            g=max(0,x-arr[pos])+max(0,y-(s-arr[pos]))
            ans=min(ans,g)
        print(ans)
