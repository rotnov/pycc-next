n = input()
n = int(n)
arr = input().split()
arr = list(map(int, arr))
print(n * 2, sum(arr))

for i in "abc":
    print(i.upper(), end=" ")
print()
for i in range(3):
    print(i * 10, end=" ")
print()

t = int(input())
for t in range(t):
    t = [x * t for x in arr]
    print(t)

c = {1, 2, 3} & {2, 3, 4}
c = sorted(c)
if c == []:
    print("empty")
print(c, c[0])

mass = input()
mass = list(mass)
mass.reverse()
print("".join(mass))


def solve(text: str) -> int:
    s = text.strip()
    total = 0
    for ch in s:
        total += ord(ch)
    s = len(s)
    return total + s


print(solve("  ab "))

total = 0
k = 5
while k > 0:
    k -= 1
    total += k
k = "done"
print(total, k)

x = 1
try:
    x = int("zz")
except ValueError:
    x = 2
print(x)
