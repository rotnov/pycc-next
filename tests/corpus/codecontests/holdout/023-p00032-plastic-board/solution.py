rect = 0
loze = 0

while True:
    try:
        n, m, o = map(int, input().split(','))
        if n ** 2 + m ** 2 == o ** 2:
            rect += 1
        if n == m:
            loze += 1
    except:
        print(rect)
        print(loze)
        break
