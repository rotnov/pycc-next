for i in range(int(input())):
    n, p, s = int(input()), [int(i) for i in input().split()], input()
    p = sorted(zip(s, p, range(n)))
    w = [0] * n
    for i in range(n):
        w[p[i][2]] = i + 1
    print(*w)
    zip()
