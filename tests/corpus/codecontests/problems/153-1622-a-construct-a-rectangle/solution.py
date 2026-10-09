for _ in range(int(input())):
    a, b, c = map(int, input().split())
    if a == b + c or b == a + c or c == a + b:
        print('YES')
    elif a == b and c % 2 == 0:
        print('YES')
    elif a == c and b % 2 == 0:
        print('YES')
    elif b == c and a % 2 == 0:
        print('YES')
    else:
        print('NO')
