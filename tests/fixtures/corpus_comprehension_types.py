import sys
input = sys.stdin.readline
square = lambda x: x * x

for _ in range(int(input())):
    a = list(map(int, input().split()))
    dp = [[100] * 4 for _ in range(3)]
    dp[0][1] = a[0]
    print(min(dp[-1]), min(dp[0]), max(dp[1]))
    words = [str(v) * 2 for v in a]
    print(words, len(words[0]))
    pos = {str(v): i for i, v in enumerate(a)}
    print(sorted(pos.items()))
    nested = [[square(v)] * 2 for v in a]
    print(nested[-1], sum(nested[0]))
    floats = [v / 2 for v in a]
    print(floats, max(floats))
