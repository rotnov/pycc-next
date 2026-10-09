
q=int(input())

for _ in range(q):
    s=str(input())
    t=str(input())
    flag=False

    for i in range(len(s)):
        for r in range(min(len(t),len(s))+1):
            l=len(t)-r+1
            if i+r<l:
                continue
            v=s[i-l+r:i+r-1]
            e=s[i:i+r]+v[::-1]
            if len(e)!=len(t):
                continue
            #print("r=",r,e,"-",v)
            if e in t:
                flag=True
                break

    if flag==True:
        print("Yes")
    else:
        print("No")
