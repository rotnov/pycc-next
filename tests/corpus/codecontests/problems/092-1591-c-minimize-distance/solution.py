for t in range(int(input())):
    n, m = map(int, input().split())
    ns = list(map(int, input().split()))
    ps = []
    nes = []
    for i in ns:
        if i > 0:
            ps.append(i)
        else:
            nes.append(i)

    total = 0
    if ps:
        ps.sort()
        for i in range(len(ps) - 1, -1, -m):
            total += ps[i] * 2
    if nes:
        nes.sort()
        nes = nes[::-1]
        for i in range(len(nes) - 1, -1, -m):
            total -= nes[i] * 2
    if ps == []:
        total += nes[-1]
    elif nes == []:
        total -= ps[-1]
    else:
        total -= max(-nes[-1], ps[-1])
    print(total)
