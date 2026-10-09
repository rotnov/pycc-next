from collections import defaultdict
mo = 998244353
def solve(n,A):
    d_minus = defaultdict(int)
    d_all = defaultdict(int)
    d_all[-1] = 1
    cnt = 0
    for a in A:
        # if a==0 and d_all[0] == 0:
        #     d_all[0] = 1
        #     cnt += 1
        #     continue
        # if a==1:
        #     if d_all[0] == 0 and d_minus[1]==0:
        #         d_minus[1] = 1
        #         cnt += 1
        #         continue
        cnt += d_all[a]
        d_all[a] = (d_all[a] * 2)%mo
        cnt += d_minus[a]
        d_minus[a] = (d_minus[a]*2)%mo
        cnt += d_all[a-2]
        d_minus[a] = (d_minus[a] + d_all[a-2])%mo
        cnt += d_all[a-1]
        d_all[a] = (d_all[a] + d_all[a-1])%mo
        cnt += d_minus[a+2]
        d_minus[a+2] = (d_minus[a+2]*2)%mo
        # d_all[a] = (d_all[a] + d_minus[a+1])%mo
        cnt = cnt%mo
    return cnt

def main():
    ans = []
    t = int(input())
    for _ in range(t):
        n=int(input())
        A=list(map(int, input().split(' ')))
        ans.append(solve(n,A))
    for a in ans:
        print(a)


if __name__ == '__main__':
    main()
