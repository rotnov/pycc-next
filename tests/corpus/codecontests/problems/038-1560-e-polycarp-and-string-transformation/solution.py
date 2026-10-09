def func():
    t = input()
    #t = t[:-1]
    #print(t)
    s = ""
    n = len(t)
    m = 0
    
    sc = [0 for i in range(26)]
    tc = [0 for i in range(26)]
    
    for i in range(n):
        temp = ord(t[i])
        #print(temp)
        tc[temp - 97] += 1
    
    for i in range(26):
        if(tc[i]!=0):
            m += 1
    
    count = m
    
    j = n-1
    temp = []
    for m in range(count,0,-1):
        i = j
        while(i>0 and (t[i] in temp)):
            i -= 1
        temp.append(t[i])
        
        an = ord(t[i])-97
        if(tc[an] % m):
            print(-1)
            return
        sc[an] = tc[an]/m
        j = i
        
    ansl = 0
    for i in range(26):
        if(sc[i]):
            ansl += sc[i]
    #print(ansl)
    
    ans = ""
    s = t[0:int(ansl)]
    ans += s
    temp = temp[::-1]
    ans2 = "".join(temp)
    #print(ans2)
    for i in range(len(temp)):
        c = str(ans2[i])
        s = s.replace(c,"")
        ans += s
        
    #print(ans)
    if(ans!=t):
        print(-1)
        return
 
    print(t[0:int(ansl)],ans2)
        
        
t = int(input())
 
for _ in range(t):
    func()
