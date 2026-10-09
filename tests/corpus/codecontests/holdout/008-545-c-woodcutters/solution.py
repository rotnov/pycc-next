ll=lambda:map(int,input().split())
t=lambda:int(input())
ss=lambda:input()
#from math import log10 ,log2,ceil,factorial as f,gcd
#from itertools import combinations_with_replacement as cs 
#from functools import reduce
#from bisect import bisect_right as br
#from collections import Counter

n=t()
x,h=[],[]
for _ in range(n):
    a,b=ll()
    x.append(a)
    h.append(b)
if n>=2:
    c=2
    tx=x[0]
    for i in range(1,n-1):
        if x[i]-tx>h[i]:
            tx=x[i]
            c+=1
        elif x[i+1]-x[i]>h[i]:
            tx=x[i]+h[i]
            c+=1
        else:
            tx=x[i]
    print(c)
else:
    print(1)
