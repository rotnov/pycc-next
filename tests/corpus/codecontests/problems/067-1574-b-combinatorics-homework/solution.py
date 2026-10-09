t = int(input())
for i in range(t):
    s = input().split()
    a = int(s[0])
    b = int(s[1])
    c = int(s[2])
    m = int(s[3])
    res = (a-1) + (b-1) + (c-1)
    if a >= b and a >= c:
        m1 = a
        s[0] = '!'
    elif b >= a and b >= c:
        m1 = b
        s[1] = '!'
    else:
        m1 = c
        s[2] = '!'
    if s[0] != '!' and (a >= b or a >= c):
        s[0] = '!'
        m2 = a
    elif s[1] != '!' and (b >= a or b >= c):
        s[1] = '!'
        m2 = b
    else:
        s[2] = '!'
        m2 = c
    if s[0] == '!' and s[1] == '!':
        m3 = c
    elif s[0] == '!' and s[2] == '!':
        m3 = b
    else:
        m3 = a
    if res >= m and m1 - (m2 + m3) - 1 <= m:
        print('YES')
    else:
        print('NO')
