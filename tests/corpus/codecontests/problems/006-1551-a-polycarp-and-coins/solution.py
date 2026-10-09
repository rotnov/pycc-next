t = int(input())

for _ in range(t):
    n = int(input())

    c1,c2 = 0,0
    
    if n%3==0:
        c1 = n//3
        c2 = n//3 
    
    elif n%3==1:
        c1 = (n+2)//3
        c2 = c1-1
    
    else:
        c2 = (n+1)//3
        c1 = c2-1
    
    print(c1,c2)
