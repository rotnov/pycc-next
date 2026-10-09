for i in range(int(input())):
    n=int(input())
    s=str(input())
    count=0
    sm=0
    for j in range(n):
        if(s[j]!='0'):
            if(s[n-1]!='0' and j==n-1):
                
                
                sm+=int(s[j])
            
            else:
                count+=1 
                sm+=int(s[j])
    
    print(sm+count)
