import math
import collections
# 試し割法

N, P = map(int, input().split())


def trial_division(n):
    # 素因数を格納するリスト
    factor = []
    # 2から√n以下の数字で割っていく
    tmp = int(math.sqrt(n)) + 1
    for num in range(2, tmp):
        while n % num == 0:
            n //= num
            factor.append(num)
    # リストが空ならそれは素数
    if not factor:
        return [n]
    else:
        factor.append(n)
        return factor


lst = trial_division(P)
c_lst = collections.Counter(lst)

ans = 1
for k, v in c_lst.items():
    while v >= N:
        ans *= k
        v -= N
print(ans)
