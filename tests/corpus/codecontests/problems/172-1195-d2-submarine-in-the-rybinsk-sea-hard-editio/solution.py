from collections import Counter

n = int(input())
a = list(map(int, input().split()))
l = [len(str(i)) for i in a]
c = Counter(l)
cl = [c[i] for i in range(1,11)]
M = 998244353

pad = lambda a, d: a%d + (a - a%d) * 10

#print(a, l, c, cl)

ans = 0

for i in a:
    il = len(str(i)) # let's calculate it again to avoid zip and enumerate
    #print('processing', i, ans)
    t = i
    for p in range(10):
        #if not cl[p]: continue
        i = pad(i, 100**p)
        #print('top pad', p, 'is', i, 'there are', cl[p])
        ans = (ans + i * cl[p]) % M
    i = t # restore
    for p in range(10):
        #if not cl[p]: continue
        i = pad(i, 10 * 100**p)
        #print('bottom pad', p, 'is', i, 'there are', cl[p])
        ans = (ans + i * cl[p]) % M

print(ans)
