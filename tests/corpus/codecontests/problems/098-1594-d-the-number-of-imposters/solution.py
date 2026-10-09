import sys
input  = sys.stdin.readline

map_role = {"imposter": 1, "crewmate": 0}
for _ in range(int(input())):
    n,m = input().split(' ')
    nocontra = True
    n= int(n)
    m= int(m)
    edges = [[] for i in range(n+1)]
    for com in range(m):
        i,j,c = input().split(' ')
        i = int(i)
        j = int(j)
        c = map_role[c.strip()]
        edges[i].append((j,c))
        edges[j].append((i,c))
    role  = [-1] *(n+1)
    res =0 
    for player in range(1,n+1):
        if role[player] == -1:
            role[player] = 0
            count  = [1,0]
            queue = [player]
            while (len(queue)>0):
                u = queue.pop()
                for edge in edges[u]:
                    v= edge[0]
                    status = edge[1]
                    if role[v] == -1:
                        role[v] = role[u] if status == 0 else 1-role[u]
                        count[role[v]]+=1
                        queue.append(v)
                    else:
                        ans = role[u] if status == 0 else 1-role[u]
                        if (role[v]!=ans):
                            nocontra = False
                            break
                if not nocontra : break
            if (not nocontra):break
            res += max(count)
    if (not nocontra) :
        print(-1)
    else: print(res)
