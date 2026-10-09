import math
n = int(input())
count = 0
for i in range(n):
    t = int(input())
    a = int(t ** (1 / 2))
    end = 0
    for j in range(2, a + 1):
        if t % j == 0:
            end = 1
            break
    if end == 0:
        count += 1
print(count)
