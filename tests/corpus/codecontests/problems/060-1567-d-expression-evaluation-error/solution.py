
def solver(s, n):
    num = 10 ** (len(str(s))-1)
    for i in range(n-1):
        while s - num < n - (i + 1):
            num //= 10
        print(num, end=' ')
        s -= num
    print(s)

T = int(input())
for t in range(T):
    S, N = map(int, input().split())
    solver(S, N)

'''
6
97 2
17 1
111 4
100 2
10 9
999999 3
1
14 7
'''
