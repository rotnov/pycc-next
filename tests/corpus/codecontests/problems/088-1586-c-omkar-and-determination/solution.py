import sys
input = iter(sys.stdin.read().splitlines()).__next__


n, m = map(int, input().split())
columns = [[] for _ in range(m)]
for _ in range(n):
    for col, cell in zip(columns, input()):
        col.append(cell)
# single column always determinable
prefix_undeterminable_pairs = [0]
for j in range(1, m):
    for i in range(n-1):
        if columns[j][i] == 'X' and columns[j-1][i+1] == 'X':
            prefix_undeterminable_pairs.append(prefix_undeterminable_pairs[-1] + 1)
            break
    else:
        prefix_undeterminable_pairs.append(prefix_undeterminable_pairs[-1])
q = int(input())
a = []
for _ in range(q):
    x_1, x_2 = map(lambda x: int(x)-1, input().split())
    a.append("YES" if prefix_undeterminable_pairs[x_1] == prefix_undeterminable_pairs[x_2] else "NO")
print("\n".join(a))
