import sys
input =sys.stdin.buffer.readline
from collections import defaultdict
from bisect import bisect_left,bisect_right
from itertools import accumulate
for _ in range(int(input())):
    n,q=map(int,input().split())
    arr=list(input())
    for i in range(n):
        arr[i] =chr(arr[i])
    for i in range(n):
        if i % 2==0:
            if arr[i] =='+':
                arr[i] =1
            else:
                arr[i] =-1
        else:
            if arr[i] =='-':
                arr[i] =1
            else:
                arr[i] =-1
    dct=defaultdict(list)
    brr=[0] +list(accumulate(arr))
    for i in range(n):
        dct[brr[i] +brr[i+1]].append(i+1)
    for i in range(q):
        l,r=map(int,input().split())
        c=brr[r]-brr[l-1]
        if c ==0:
            print(0)
            continue
        if (r-l+1) % 2==0:
            d=brr[r] +brr[l]
            bb=dct[d][bisect_left(dct[d],l+0.5)]
            print(2)
            print(l,bb)
        else:
            d=brr[r] +brr[l-1]
            bb=dct[d][bisect_left(dct[d],l-0.5)]
            print(1)
            print(bb)
