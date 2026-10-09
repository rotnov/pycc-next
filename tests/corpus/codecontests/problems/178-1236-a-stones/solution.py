t = int(input())

while t>0:
    
    x, y, z = [int(i) for i in input().split()]
    
    s = 0
    f = -1
    z = z//2
    if y >= z: 
        y = y - z
        s = z*2 + z
    else:
        s = y*2 + y
        f = 1
    
    if f == -1:
        y = y//2
        if x >= y:
            s = s + 2*y + y
        else:
            s = s + 2*x + x

    print(s)
    
    t-=1
