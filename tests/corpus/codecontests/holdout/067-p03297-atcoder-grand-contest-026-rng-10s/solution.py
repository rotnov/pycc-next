def main():
    import math

    def gcd(a, b):
        while b:
            a, b = b, a % b
        return a
    N = int(input())
    ABCD = [list(map(int, input().split())) for i in range(N)]
    for A,B,C,D in ABCD:
        # 在庫が買う本数以下 or 在庫追加が買う本数以下
        if A < B or D < B:
            print("No")
            continue

        A %= B
        # 在庫が買う本数以下になり、補給出来ない
        if C < A:
            print("No")
            continue
        if B == D:
            print("Yes")
            continue

        flag = True
        #print((B-A-1)%(D-B) ,">", (B-1)-(C+1))
        if (B-A-1)%(gcd(D,B)) <= (B-1)-(C+1):
            flag = False
        
        print("Yes" if flag else "No")


if __name__ == '__main__':
    main()
