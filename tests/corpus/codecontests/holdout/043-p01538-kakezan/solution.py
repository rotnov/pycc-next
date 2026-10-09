"かけざん"
"最大のものを取得して、一桁になるまでに操作を行う回数を答える"


def kakezan(n):
    ret = 0
    str_n = str(n)
    digit_amount = len(str_n)
    for i in range(digit_amount-1):
        # print(str_n[:i+1])
        # print(str_n[i+1:])
        # print("")
        ret = max(ret, int(str_n[:i+1])*int(str_n[i+1:]))

    return ret


Q = int(input())  # 入力される整数の個数
N = [int(input()) for i in range(Q)]

for n in N:
    cnt = 0
    while n >= 10:
        n = kakezan((n))
        cnt += 1
    print(cnt)
