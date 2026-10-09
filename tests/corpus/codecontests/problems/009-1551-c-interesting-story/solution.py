from collections import Counter
t = int(input())
for _ in range(t):
    n = int(input())
    ls = ['a', 'b', 'c', 'd', 'e']
    pwrt = [[], [], [], [], []]
    for i in range(n):
        s = input()
        q = len(s)
        c = Counter(s)
        for i, x in enumerate(ls):
            pwrt[i].append(2 * c[x] - q)
    h = set()
    for i in range(5):
        pwrt[i].sort(reverse=True)
        # print(pwrt[i])
        j, s = 0, 0
        while j < n and s + pwrt[i][j] > 0:
            s += pwrt[i][j]
            j += 1
        h.add(j)
    print(max(h))
