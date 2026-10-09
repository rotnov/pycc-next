# cook your dish here
t=int(input())
for _ in range(t):
    n=int(input())
    a=list(map(int,input().split()))
    l=[0]*n
    d={}
    f=0
    for i in range(n):
        l[i]=i+a[i%n]
        d[l[i]]=d.get(l[i],0)+1
        if d[l[i]]==2:
            f=1
            break
    r={}
    for i in range(n):
        r[l[i]%n]=r.get(l[i]%n,0)+1
        if r[l[i]%n]==2:
            f=1
            break
    if f:
        print('NO')
    else:
        print('YES')
