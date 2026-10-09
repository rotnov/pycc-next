def solve(a, b):
    if a == 0:
        return False
    if solve(b % a, a):
        b //= a
        return not (b % (a + 1) & 1)
    return True


n = int(input())
for _ in range(n):
    a, b = [int(x) for x in input().split()]

    if a > b:
        a, b = b, a

    if solve(a, b):
        print("First")
    else:
        print("Second")
