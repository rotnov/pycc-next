for _ in range(int(input())):
    
    n = int(input())
    a = list(map(int,input().split()))
    
    b =  []
    mn = -1e18
    for i in range(n-1):
        
        if(a[i] == -1 and a[i+1] == -1):
            continue
        elif(a[i] == -1):
            b.append(abs(0 - a[i+1]))
        elif(a[i+1]==-1):
            b.append(abs(0 - a[i]))
        elif(a[i+1] != -1 and a[i] != -1):
            mn = max(abs(a[i+1] - a[i]),mn)
    
    b.sort()
    if(len(b) == 0):
        print(0,0)
        continue
    
    
    md = (b[0]+b[-1])//2
    # print(b)
    
    
    for i in range(n-1):
        if(a[i] != -1 and a[i+1] != -1):
            mn = max(abs(a[i+1] - a[i]),mn)
        elif(a[i] == -1 and a[i+1] == -1):
            mn = max(mn,0)
        elif(a[i] == -1 and a[i+1] !=-1):
            mn = max(mn,abs(a[i+1] - md))
        else:
            mn = max(mn,abs(a[i] - md))
            
    print(mn,md)
