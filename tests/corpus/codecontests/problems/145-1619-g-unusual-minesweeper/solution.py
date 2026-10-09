from sys import stdin
input=stdin.readline

class dsu:

    def __init__(self , n):

        self.p = [0]*(n + 1)
        self.rank = [0]*(n + 1)

        for i in range(1 , n + 1):
            self.p[i] = i

    def find(self , node):
        if(self.p[node] == node):return node

        self.p[node] = self.find(self.p[node])
        return self.p[node]

    def union(self , u , v):
        u , v = self.find(u) , self.find(v)

        if(self.rank[u] == self.rank[v]):
            self.p[v] = u
            self.rank[u] += 1

        elif(self.rank[u] > self.rank[v]):
            self.p[v] = u

        else:
            self.p[u] = v

def solve(a , n):

    j = 0
    for i in range(n):

        while(j < n and a[j][0] <= (a[i][0] + k)):

            x , y = comp.find(a[i][1]) , comp.find(a[j][1])
            if(x != y):
                time[x] = min(time[x] , time[y])
                time[y] = min(time[y] , time[x])

            comp.union(a[i][1] , a[j][1])

            j += 1

def answer():

    for x in ax.keys():
        solve(ax[x] , len(ax[x]))

    for y in ay.keys():
        solve(ay[y] , len(ay[y]))


    allcomp = set()
    for i in range(1 , n + 1):
        allcomp.add(comp.find(i))
   
    value = []
    for i in allcomp:
        value.append(time[i])

    value.sort(reverse = True)

    ans = -1
    for i in range(len(value)):
        if(i > value[i]):break
        ans += 1

    return ans

for T in range(int(input())):

    input().strip()

    n , k = map(int,input().split())

    ax  , ay = dict() , dict()
    time = [0] * (n + 1)
    for i in range(n):
        x , y , timeval = map(int,input().split())
        ax[x] = ax.get(x , []) + [[y , i + 1 , timeval]]
        ay[y] = ay.get(y , []) + [[x , i + 1 , timeval]]

        time[i + 1] = timeval

    for x in ax.keys():
        ax[x].sort()

    for y in ay.keys():
        ay[y].sort()

    comp = dsu(n)

    print(answer())

            
