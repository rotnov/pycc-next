#!/usr/bin/env python3

n, k, p = map(int, input().strip().split())

if k == 0:
    ak = 0
    an = n
else:
    ak = k - 1 if n % 2 == 1 else k
    an = n - (n % 2)

ans = ''
for i in range(p):
    v = int(input().rstrip())
    if k == 0:
        print('.', end='')
    else:
        if v == n:
            print('X', end='')
        else:
            idx = (an - v) / 2
            idx += (v % 2) * (an / 2)
            if idx >= ak:
                print('.', end='')
            else:
                print('X', end='')
