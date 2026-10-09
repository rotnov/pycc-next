import sys
input = sys.stdin.readline

n,m = list(map(int,input().split()))
a = [0]*(n+1)
ans = n
for i in range(m):
    u,v = list(map(int,input().split()))
    u = min(u,v)
    if a[u]==0:
        ans-=1
    a[u]+=1

q = int(input())
for i in range(q):
    b = list(map(int,input().split()))
    if b[0]==1:
        b[1] = min(b[1],b[2])
        if a[b[1]]==0:
            ans-=1
        a[b[1]]+=1
    elif b[0]==2:
        b[1] = min(b[1],b[2])
        a[b[1]]-=1
        if a[b[1]]==0:
            ans+=1
    else:
        print(ans)
        
