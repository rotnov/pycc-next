n=int(input())
p=[complex(*map(int,input().split())) for _ in range(n)]
e=[b-a for a,b in zip(p,p[1:]+[p[0]])]
p=e[0]
for i in range(1,n,):
    now=e[-i]
    if now.real*p.imag-p.real*now.imag<-1e-6:print(0);break
    p=now
else: print(1)
