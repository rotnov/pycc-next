for _ in range(int(input())):
    n = int(input())
    s = list(map(int,list(input())))
    m = list(map(int,list(input())))
    ans = 0
    if m[0] == 1:
        if s[0]==0: ans +=1
        elif s[1] == 1: ans +=1; s[1] = 2
    for x in range(1, n):
        if m[x] == 1:
            if s[x] == 0:
                ans +=1
            elif s[x-1] == 1:
                ans += 1
            elif x+1 != n and s[x+1] == 1:
                ans +=1
                s[x + 1] = 2
    print(ans)
