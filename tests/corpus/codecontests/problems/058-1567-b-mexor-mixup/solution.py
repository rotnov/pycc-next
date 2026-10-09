def xor_culc(n) :

    if n % 4 == 0 :
        return n
 
    if n % 4 == 1 :
        return 1

    if n % 4 == 2 :
        return n + 1
 
    return 0
    
for _ in range(int(input())):
    a, b = map(int, input().split())
    
    num = xor_culc(a-1)
    if num == b: print(a)
    elif num ^ b == a: print(a+2)
    else: print(a+1)
