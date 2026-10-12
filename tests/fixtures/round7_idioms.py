# Idioms of competitive-programming code, compiled natively.
import sys

# A line reader over input read up front; StopIteration once exhausted.
input = iter(sys.stdin.read().splitlines()).__next__

n = int(input())
rows = [list(map(int, input().split())) for _ in range(n)]

# `.pop()` of a subscripted list.
grid = [[1, 2, 3], [4], [5, 6]]
print(grid[0].pop(), grid[2].pop(), grid)

# List displays as unpacking targets, nested targets in `for`.
for i, [a, b] in enumerate(rows):
    print(i, a + b)
for k, (p, q) in enumerate([(1, 2), (3, 4)], 1):
    print(k, p * q)
[x, y] = rows[0]
print(x, y)

# A tuple display only iterated or searched.
total = 0
for z in (x, y, 10):
    total += z
print(total, 3 in (x, y), 4 not in (x, y), [w * 2 for w in (x, y)])

print(input())
print(input())
