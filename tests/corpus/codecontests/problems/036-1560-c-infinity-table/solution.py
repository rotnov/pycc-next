for _ in range(int(input())):
    k = int(input())
    sr = int((k - 1) ** .5)
    r = k - sr * sr
    if r <= sr:
        print(r, sr + 1)
    else:
        print(sr + 1, sr + sr + 2 - r)
