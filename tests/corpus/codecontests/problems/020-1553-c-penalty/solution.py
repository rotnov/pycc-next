for i in range(int(input())):
    s=input()
    n=10
    a=0
    b=0
    p1=0
    p2=0
    z1=0
    z2=0
    for i in range(10):
        if i%2==0:
            if s[i]=='?':a+=1
            else:p1+=int(s[i])
        else:
            if s[i]=='?':b+=1
            else:p2+=int(s[i])
        if i%2==0:z1=1;z2=0
        else:z1=0;z2=1
        if a+p1>p2+(n-i-z2)//2:print(i+1);break
        elif b+p2>p1+(n-i-z1)//2:print(i+1);break
    else:print(10)
