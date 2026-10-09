def find(i, total, k):
    if i == n:
        if total == 0 and k > 0:
            return True
        return False
    return find(i + 1, total + a[i], k + 1) or find(i + 1, total - a[i], k + 1) or find(i + 1, total, k)


def sex(n, a):
    if n == 1:
        if a[0] == 0:
            return 'YES'
        return 'NO'
    a = [abs(a[i]) for i in range(n)]
    a.sort()
    for i in range(n - 1):
        if a[i] == a[i + 1]:
            return 'YES'
    if find(0, 0, 0):
        return 'YES'
    return 'NO'



t = int(input())
for _ in range(t):
    n = int(input())
    a = list(map(int, input().split()))
    print(sex(n, a))
