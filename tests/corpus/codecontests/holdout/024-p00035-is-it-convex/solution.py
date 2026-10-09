def gai(a,b,c,d):
    S = a*d - b*c
    return(S)
while True:
    try:
        x1,y1,x2,y2,x3,y3,xp,yp  = map(float,input().split(","))
        A1,A2,B1,B2,C1,C2,D1,D2 = x1,y1,x2,y2,x3,y3,xp,yp
        
        if gai(x1 - x2,y1 - y2,x1 - xp,y1 - yp) < 0 and gai(x2 - x3,y2 - y3,x2 - xp,y2 - yp) < 0 and gai(x3 - x1,y3 - y1,x3 - xp,y3 - yp) < 0:
            print("NO")
        elif gai(x1 - x2,y1 - y2,x1 - xp,y1 - yp) > 0 and gai(x2 - x3,y2 - y3,x2 - xp,y2 - yp) > 0 and gai(x3 - x1,y3 - y1,x3 - xp,y3 - yp) > 0:
            print("NO")
        else:
            x1,y1,x2,y2,x3,y3,xp,yp = A1,A2,B1,B2,D1,D2,C1,C2
            if gai(x1 - x2,y1 - y2,x1 - xp,y1 - yp) < 0 and gai(x2 - x3,y2 - y3,x2 - xp,y2 - yp) < 0 and gai(x3 - x1,y3 - y1,x3 - xp,y3 - yp) < 0:
                print("NO")
            elif gai(x1 - x2,y1 - y2,x1 - xp,y1 - yp) > 0 and gai(x2 - x3,y2 - y3,x2 - xp,y2 - yp) > 0 and gai(x3 - x1,y3 - y1,x3 - xp,y3 - yp) > 0:
                print("NO")
            else:
                x1,y1,x2,y2,x3,y3,xp,yp = A1,A2,D1,D2,C1,C2,B1,B2
                if gai(x1 - x2,y1 - y2,x1 - xp,y1 - yp) < 0 and gai(x2 - x3,y2 - y3,x2 - xp,y2 - yp) < 0 and gai(x3 - x1,y3 - y1,x3 - xp,y3 - yp) < 0:
                    print("NO")
                elif gai(x1 - x2,y1 - y2,x1 - xp,y1 - yp) > 0 and gai(x2 - x3,y2 - y3,x2 - xp,y2 - yp) > 0 and gai(x3 - x1,y3 - y1,x3 - xp,y3 - yp) > 0:
                    print("NO")
                else:
                    x1,y1,x2,y2,x3,y3,xp,yp = D1,D2,B1,B2,C1,C2,A1,A2
                    if gai(x1 - x2,y1 - y2,x1 - xp,y1 - yp) < 0 and gai(x2 - x3,y2 - y3,x2 - xp,y2 - yp) < 0 and gai(x3 - x1,y3 - y1,x3 - xp,y3 - yp) < 0:
                        print("NO")
                    elif gai(x1 - x2,y1 - y2,x1 - xp,y1 - yp) > 0 and gai(x2 - x3,y2 - y3,x2 - xp,y2 - yp) > 0 and gai(x3 - x1,y3 - y1,x3 - xp,y3 - yp) > 0:
                        print("NO")
                    else:
                        print("YES")

            
    except EOFError:
        break
