for _ in range(int(input())):
    n=int(input())
    s=list(input().strip())
    temp2=sorted(s)
    count=0
    for i in range(n):
        if s[i]!=temp2[i]:
            count+=1
    print(count)        
                  
                  
    
