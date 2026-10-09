a=input()
while True:
    try:
        t=int(input()) 
        a=[int(i) for i in input().split()]
        n_0=a.count(0)
        n_1=a.count(1)
        #print(n_0,n_1)
        ans=0
        #if n_1>1:
            #n_1-=1
        ans+=n_1*2**n_0
        if sum(a)==1:
            ans=2**(n_0)
        print(ans)
    except:
        break
