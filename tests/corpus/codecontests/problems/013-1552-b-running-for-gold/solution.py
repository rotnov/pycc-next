t = int(input().strip())
while(t):
    t-=1
    n = int(input().strip())
    a = []
    for i in range(n):
        a.append(list(map(int ,input().strip().split())))

    if n==1:
        print(1)
        continue
    
    res = -1
    gold = 0        # 0 is index of first player assuming he is superior

    for i in range(1,n):
        num = 0
        for j in range(5):
            if(a[i][j]<a[gold][j]):
                num+=1
        if(num>=3):
            gold = i        

    i = 0
    for i in range(0,n):
        num = 0
        if(i!=gold):
            for j in range(5):
                if(a[i][j]<a[gold][j]):
                    num+=1
            if(num>=3):
                res = -1
                break
    if(i==n-1):
        res = gold+1

    print(res)                            


    # ath_rec = []

    # for i in range(n):
    #     ath_rec.append(list(map(int, input().strip().split())))

    # # so ath_rec is a (N*5) matrix now we check for each column 

    # superior = [0]*(n+1)

    # for k in range(5):
    #     for i in range(n):
    #         for j in range(n):
    #             if(ath_rec[i][k]<ath_rec[j][k]):
    #                 superior[i+1] +=1

    # print(superior)                
        



    
