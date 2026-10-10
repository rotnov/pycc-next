import bisect
import heapq

sr = lambda: input()
lr = lambda: list(map(int, sr().split()))

# A helper's inferred return, through a chained lambda, feeds a heap of
# tuples built with unary minus.
a = lr()
h = []
for ind, num in enumerate(a):
    heapq.heappush(h, (-num, ind + 1))
while len(h) > 1:
    num1, ind1 = heapq.heappop(h)
    num2, ind2 = heapq.heappop(h)
    print(ind1, ind2)
    num1 += 1
    if num1 < 0:
        heapq.heappush(h, (num1, ind1))
print(h)

# `sorted` (plain and keyed) of a list typed by its appends, then unpacked
# by the loop target into another empty list.
temp = []
for i in range(len(a)):
    temp.append([a[i], i + 1 - a[i]])
temp = sorted(temp, key=lambda x: [x[0], -x[1]])
seq = []
for [p, d] in temp:
    if d < 0:
        continue
    loc = bisect.bisect(seq, d)
    if loc == len(seq):
        seq.append(d)
    else:
        seq[loc] = d
print(len(seq), seq)

pairs = []
for i in range(3):
    pairs.append((i, str(i)))
names = []
for n, s in sorted(pairs, reverse=True):
    names.append(s * n)
print(names)

# A comprehension over a list typed later, and `~`.
flipped = []
for x in [y * 2 for y in a]:
    flipped.append(~x)
print(flipped)
