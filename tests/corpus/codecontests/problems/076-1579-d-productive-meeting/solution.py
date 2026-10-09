import heapq
sr=lambda: input()
ir=lambda: int(sr())
lr=lambda: list(map(int, sr().split()))

inf=10**18
# mod=10**9+7
mod = 998244353

if __name__=='__main__':
    test = ir()
    for t in range(test):
        n=ir()
        a=lr()
        h = []
        ans = []
        for ind,num in enumerate(a):
            if num > 0:
                heapq.heappush(h, (-num, ind+1))
        while len(h) > 1:
            num1, ind1 = heapq.heappop(h)
            num2, ind2 = heapq.heappop(h)
            ans.append([ind1, ind2])
            num1+=1
            num2+=1
            if num1 < 0:
                heapq.heappush(h, (num1, ind1))
            if num2 < 0:
                heapq.heappush(h, (num2, ind2))
        print(len(ans))
        for l in ans:
            print(*l, sep=' ')
