t = int(input())
for _ in range(t):
    n = int(input())
    a = input()
    b = input()
    for i in range(n):
        if a[i] == '1' and b[i] == '1':
            print('NO')
            break
    else:
        print('YES')
