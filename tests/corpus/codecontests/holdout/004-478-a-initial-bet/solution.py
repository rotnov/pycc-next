l=list(map(int,input().split()))
x=sum(l)
if(x%5==0 and x!=0):
    print(int(x/5))
else:
    print(-1)
