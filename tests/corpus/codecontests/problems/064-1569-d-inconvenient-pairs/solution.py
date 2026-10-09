import sys
import math
from collections import deque,Counter
from sys import stdin

#sys.setrecursionlimit(10**7)

int1=lambda x: int(x)-1
stinput=lambda :stdin.readline()[:-1]
ii=lambda :int(stinput())
mi=lambda :map(int, stdin.readline().split())
li=lambda :list(mi())
mi1=lambda :map(int1, stdin.readline().split())
li1=lambda :list(mi1())
mis=lambda :map(str, stdin.readline().split())

pr=print
rg=range

from collections import defaultdict
"""
#初期値 0
d=defaultdict(int)

#初期値 1
d=defaultdict(lambda:1)
"""



mod=10**9+7
Mod=998244353
INF=10**18
ans=0
num=2**30

t=ii()
for _ in range(t):
  n,m,k=mi()
  x=li()
  y=li()
  sx=set(x)
  sy=set(y)
  a=[]
  b=[]
  for i in x:
    b.append(i*num+2**20)
  for i in y:
    a.append(i*num+2**20)
  for i in range(k):
    p,q=mi()
    if p in sx:
      if q not in sy:
        a.append(num*q+p)
    else:
      b.append(num*p+q)
  a.sort()
  b.sort()
  ans=0
  
  d=defaultdict(int)
  tmp=0
  for i in a:
    p,q=i//num,i%num
    if q==2**20:
      d=defaultdict(int)
      ans+=tmp*(tmp-1)//2
      tmp=0
    else:
      ans-=d[q]
      d[q]+=1
      tmp+=1
  
  d=defaultdict(int)
  tmp=0
  for i in b:
    p,q=i//num,i%num
    if q==2**20:
      d=defaultdict(int)
      ans+=tmp*(tmp-1)//2
      tmp=0
    else:
      ans-=d[q]
      d[q]+=1
      tmp+=1
  
  print(ans)
