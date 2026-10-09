for _ in range(int(input())):
    n,k  = map( int, input().split(' ') )
    arr = [int(w) for w in input().split(' ')]
    ans = -10**18
    temp =[]
    cnt = 0 
    for i in range(n-1,-1,-1):
        temp.append( (arr[i],i) )
        cnt += 1 
        if cnt==300:
            break 
    for i in range(len(temp)):
        for j in range(i+1,len(temp)):
            u1,v1 = temp[i]
            u2,v2 = temp[j]
            ans = max(ans ,  (v1+1)*(v2+1) - k*(u1|u2)  )
    print(ans)
