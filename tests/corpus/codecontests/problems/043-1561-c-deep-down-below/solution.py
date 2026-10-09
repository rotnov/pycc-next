
# from math import gcd

t = int(input())

for _ in range(t):
    n = int(input())
    ar2 = []
    ar4 = []
    final = 5000000000
    initialfinal = 50000000000
    cpower = 0
    clen = 0
    index = 0
    minind = 0
    for i in range(n):
        ar2 = list(map(int,input().split()))
        ar2 = ar2[1:]
        # mx = max(ar2)
        # ind = ar2.index(mx)
        # ans = mx - ind
        # if(ans < final):
        #     final = ans
        #     ar3 = ar2
        power = 0
        initial = 0
        for i in range(len(ar2)):
            if (ar2[i] >= power):
                power = ar2[i] + 1
                if(power-i > initial):
                    initial = power-i
            power += 1

        ar4.append([initial,len(ar2)])

        if(initial < initialfinal):
            initialfinal = initial
            minind = index

        index += 1

    ar4 = sorted(ar4)
    cpower = initialfinal
    for i in range(n):
        if(cpower >= ar4[i][0]):
            cpower += ar4[i][1]
        else:
            initialfinal += (ar4[i][0] - cpower)
            cpower += ar4[i][1] + (ar4[i][0] - cpower)

    print(initialfinal)


    # if(maxind == minind):
    #     print(initialfinal if initialfinal >= 0 else 0)
    # else:
    #     maxpower = initialfinal
    #     for i in range(n):
    #         if(i == maxind):
    #             continue
    #         maxpower += ar3[i]
    #     if(maxpower >= maxinitial):
    #         print(initialfinal if initialfinal >= 0 else 0)
    #     else:
    #         c = maxinitial - maxpower
    #         initialfinal += (c)
    #         print(initialfinal if initialfinal >= 0 else 0)
    #
