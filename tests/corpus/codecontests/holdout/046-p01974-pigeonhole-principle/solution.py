import itertools

n = int(input())
num = list(map(int, input().split()))

for comb in itertools.combinations(num, 2):
    diff = abs(comb[0] - comb[1])
    if diff % (n-1) == 0:
        print(comb[0], comb[1])
        break
