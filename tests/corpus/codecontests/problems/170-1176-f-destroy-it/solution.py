from sys import stdin, stdout, exit

n = int(input())

inf = 10**18
dp = [[-inf]*10 for i in range(n+1)]
dp[0][0] = 0
for i in range(n):
    k = int(stdin.readline())
    cards = []
    for j in range(k):
        c, d = map(int, stdin.readline().split())
        cards.append((c, d))
    cards.sort(reverse=True)
    cards_by_cost = [[] for i in range(3)]
    for c,d in cards:
        cards_by_cost[c-1].append(d)
#    print(cards_by_cost)
    for j in range(3):
        cards_by_cost[j] = cards_by_cost[j][:3]
    for prev_played in range(10):
        val = dp[i][prev_played]
        dp[i+1][prev_played] = max(dp[i+1][prev_played], val)
        for num_played in range(len(cards_by_cost[0])):
            pld = num_played+prev_played+1
            if pld >= 10:
                dp[i+1][pld%10] = max(dp[i+1][pld%10], val + sum(cards_by_cost[0][:num_played+1]) + cards_by_cost[0][0])
            else:
                dp[i+1][pld] = max(dp[i+1][pld], val + sum(cards_by_cost[0][:num_played+1]))
        if len(cards_by_cost[1]) > 0 and len(cards_by_cost[0]) > 0:
            pld = 2 + prev_played
            c0 = cards_by_cost[0][0]
            c1 = cards_by_cost[1][0] 
            if pld >= 10:
                dp[i+1][pld%10] = max(dp[i+1][pld%10], val + c0 + c1+ max(c0, c1))
            else:
                dp[i+1][pld] = max(dp[i+1][pld], val+c0+c1)
        if len(cards_by_cost[1]) > 0:
            pld = 1+prev_played
            if pld >= 10:
                dp[i+1][pld%10] = max(dp[i+1][pld%10], val+2*cards_by_cost[1][0])
            else:
                dp[i+1][pld] = max(dp[i+1][pld], val + cards_by_cost[1][0])
        if len(cards_by_cost[2]) > 0:
            pld=1+prev_played
            if pld >= 10:
                dp[i+1][pld%10] = max(dp[i+1][pld%10], val+2*cards_by_cost[2][0])
            else:
                dp[i+1][pld] = max(dp[i+1][pld], val + cards_by_cost[2][0])

ans = max(dp[n][i] for i in range(10))
stdout.write(str(ans) + "\n")
