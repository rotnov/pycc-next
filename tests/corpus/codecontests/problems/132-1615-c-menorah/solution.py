for t in range(int(input())):
    n = int(input())
    a = input()
    b = input()
    _1_1 = _0_0 = _1_0 = _0_1 = 0
    
    if a == b:
        print(0)
    elif int(b) == 0 or int(a) == 0:
        print(-1)
    
    else:
        for i, j in (zip(a, b)):
            if i == j:
                if i == '1':
                    _1_1 += 1
                else:
                    _0_0 += 1
            else:
                if i == '1':
                    _1_0 += 1
                else:
                    _0_1 += 1
        s = _1_1 + _0_0
        s1 = _1_0 + _0_1
        if _1_1 - _0_0 == 1 and _1_0 == _0_1:
            print(min(s, s1))
        elif _1_1 - _0_0 == 1:
            print(s)
        elif _1_0 == _0_1:
            print(s1)
        else:
            print('-1')
   
            
            
