# `sys.stdin.buffer.readline` reads bytes; an alias whose every call is
# converted to numbers at once reads the same as text, natively.
import sys
input = sys.stdin.buffer.readline
t = int(input())
for _ in range(t):
    n, m = map(int, input().split())
    xs = [int(x) for x in input().split()]
    print(n + m, sum(xs), float(input()))
