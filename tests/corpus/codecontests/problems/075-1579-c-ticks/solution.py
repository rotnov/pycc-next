t=int(input())
while t:
    t-=1
    nums=list(map(int,input().split()))
    n,m,k=nums[0],nums[1],nums[2]
    graph=[[] for _ in range(n)]
    points=set()
    for i in range(n):
        word=input()
        for j in range(m):
            graph[i].append(word[j])
            if word[j]=='*':points.add((i,j))
    
    check=set()
    qualified=set()
    def ops(radius):
        for posx in range(n):
            for posy in range(m):
                #print(posx,posy,n)
                if posy-radius<0 or posy+radius>=m:continue
                if posx-radius<0:continue
                if graph[posx][posy] != '*':continue
                flag1=all(graph[posx-h][posy-h]=='*' for h in range(1,radius+1))
                flag2=all(graph[posx-h][posy+h]=='*' for h in range(1,radius+1))
                if flag1 and flag2:
                    if (posx,posy) not in qualified:qualified.add((posx,posy))
                    else:continue
                    for h in range(radius+1):
                        check.add((posx-h,posy-h))
                        check.add((posx-h,posy+h))
        return
    
    for i in range(min(n,m),k-1,-1):
        ops(i)
    #print(points)
    #print(check)
    
    print(['NO','YES'][check==points])
