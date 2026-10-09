while True:
    a,b = map(int, input().split())
    if a==b == 0:
        break
    for i in range(a):
        s = ""
        for j in range(b):
            s += "#" if (i+j) % 2 == 0 else "."
        print(s)

    print("")
