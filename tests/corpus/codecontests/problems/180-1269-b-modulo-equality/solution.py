#! /usr/bin/env python
# -*- coding: utf-8 -*-
out = 0


def swap():
    global out
    if a[-1] > mb:
        t = m-a[-1]+mb
    else:
        t = mb-a[-1]
    out += t
    for i in range(n):
        a[i] += t
    a.insert(0, mb)
    del a[-1]


n, m = map(int, input().split())
a = sorted(map(int, input().split()))
b = sorted(map(int, input().split()))
mb = min(b)
while a != b:
    swap()
print(out)
