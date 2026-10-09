for _ in range(int(input())):
    n, c = input().split()
    n = int(n)
    s = input()
    if s.count(c) == n:
        print(0)
    else:
        flag = 0
        for ind in range(n - 1, -1, -1):
            if s[ind] == c:
                if ind + 1 >= n // 2 + 1:
                    print(1)
                    print(ind + 1)
                    flag = 1
                    break

        if not flag:
            print(2)
            print(n - 1, n)
