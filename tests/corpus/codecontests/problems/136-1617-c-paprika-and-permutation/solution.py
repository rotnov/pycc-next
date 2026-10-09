for _ in range(int(input())):
    n = int(input())
    nums = list(map(int, input().split()))
    fnums = {}
    for x in nums:
        fnums[x] = fnums.get(x, 0) + 1
    missing = []
    for x in range(1, n+1):
        if x in fnums:
            fnums[x] -= 1
            if fnums[x] == 0:
                del fnums[x]
        else:
            missing.append(x)
    remaining = []
    for x, f in fnums.items():
        remaining.extend([x]*f)
    remaining.sort()
    if all(m <= (r-1)//2 for m, r in zip(missing, remaining)):
        print(len(missing))
    else:
        print(-1)
