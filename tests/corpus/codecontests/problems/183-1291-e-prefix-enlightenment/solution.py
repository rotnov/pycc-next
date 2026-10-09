from sys import stdin
input = stdin.readline

n , k = [int(i) for i in input().split()]
pairs = [i + k for i in range(k)] + [i for i in range(k)]
initial_condition = list(map(lambda x: x == '1',input().strip()))
data = [i for i in range(2*k)] 
constrain = [-1] * (2*k)
h = [0] * (2*k)
L = [1] * k + [0] * k
dp1 = [-1 for i in range(n)]
dp2 = [-1 for i in range(n)]
for i in range(k):
    input()
    inp = [int(j) for j in input().split()]
    for s in inp:
        if dp1[s-1] == -1:dp1[s-1] = i
        else:dp2[s-1] = i

pfsums = 0
ans = []


def remove_pfsum(s1):
    global pfsums
    if constrain[s1] == 1:
        pfsums -= L[s1]
    elif constrain[pairs[s1]] == 1:
        pfsums -= L[pairs[s1]]
    else:
        pfsums -= min(L[s1],L[pairs[s1]])

def sh(i):
    while i != data[i]:
        i = data[i]
    return i

def upd_pfsum(s1):
    global pfsums
    if constrain[s1] == 1:
        pfsums += L[s1]
    elif constrain[pairs[s1]] == 1:
        pfsums += L[pairs[s1]]
    else:
        pfsums += min(L[s1],L[pairs[s1]])

def ms(i,j):
    i = sh(i) ; j = sh(j)
    cons = max(constrain[i],constrain[j])

    if h[i] < h[j]:
        data[i] = j
        L[j] += L[i]
        constrain[j] = cons
        return j
    else:
        data[j] = i
        if h[i] == h[j]:
            h[i] += 1
        L[i] += L[j]
        constrain[i] = cons
        return i

for i in range(n):
    if dp1[i] == -1 and dp2[i] == -1:
        pass
    elif dp2[i] == -1:
        s1 = sh(dp1[i])
        remove_pfsum(s1)
        constrain[s1] = 0 if initial_condition[i] else 1
        constrain[pairs[s1]] = 1 if initial_condition[i] else 0
        upd_pfsum(s1)
    else:
        s1 = sh(dp1[i]) ; s2 = sh(dp2[i])
        if s1 == s2 or pairs[s1] == s2:
            pass
        else:
            remove_pfsum(s1)
            remove_pfsum(s2)
            if initial_condition[i]:
                new_s1 = ms(s1,s2)
                new_s2 = ms(pairs[s1],pairs[s2])
            else:
                new_s1 = ms(s1,pairs[s2])
                new_s2 = ms(pairs[s1],s2)
            pairs[new_s1] = new_s2
            pairs[new_s2] = new_s1
            upd_pfsum(new_s1)

    ans.append(pfsums)

for i in ans:
    print(i)
