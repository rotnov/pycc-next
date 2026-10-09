from heapq import heappush,heappop
from collections import deque
t=int(input())
for i in range(t):
    n=int(input())
    a=list(map(int,input().split()))
    b=list(map(int,input().split()))
    ha=[]
    hb=[]
    for i in range(n):
        heappush(ha,[-a[i],i])
        heappush(hb,[-b[i],i])
    qa=deque([])
    qb=deque([])
    x=heappop(ha)
    qb.append(x[1])
    ans=["0" for i in range(n)]
    va=[0 for i in range(n)]
    vb=[0 for i in range(n)]
    va[x[1]]=1
    ans[x[1]]="1"
    while True:
        # print(ha,hb)
        if len(qb)==0 and len(qa)==0:
            break
        if len(qb)>0:
            x=qb.popleft()
            if vb[x]==1:
                continue
            while len(hb)>0 and hb[0][1]!=x:
                y=heappop(hb)
                qa.append(y[1])
                ans[y[1]]="1"
                vb[y[1]]=1
            heappop(hb)
        if len(qa)>0:
            x=qa.popleft()
            if va[x]==1:
                continue
            while len(ha)>0 and ha[0][1]!=x:
                y=heappop(ha)
                qb.append(y[1])
                ans[y[1]]="1"
                va[y[1]]=1
            heappop(ha)
    print("".join(ans))
        
