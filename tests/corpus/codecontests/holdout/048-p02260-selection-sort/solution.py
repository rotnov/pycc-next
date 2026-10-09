n = int(input())
s = list(map(int, input().split()))

indexMin = 0
count = 0


for start in range(n):
    indexMin = s[start:].index(min(s[start:])) + start
    s[start], s[indexMin] = s[indexMin], s[start]
    if start != indexMin:
        count+=1

print(*s)
print(count)
