N=int(input())
def dfs(S):
    if len(S)==N:
        print(S)
    else:
        for i in range(97,ord(max(S))+2):
            dfs(S+chr(i))
dfs("a")
