for _ in range(int(input())):
    n, a = int(input()), list(map(int, input().split()))
    if len(set(a)) < n: print("YES"); continue
    sorted_a = sorted(a)
    
    num2i = {num: i for i, num in enumerate(sorted_a)}
    vis = [0] * n
    ans = 0
    for i in range(n):
        if vis[i]: continue
        size, cur = 0, i
        while not vis[cur]:
            vis[cur] = 1
            size, cur = size + 1, num2i[a[cur]]
        ans += (size % 2 == 0)
    
    print("YES") if ans == 0 or ans % 2 == 0 else print("NO")
    
