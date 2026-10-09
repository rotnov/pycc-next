import bisect

n = int(input())

temp = []

arr = list(map(int,input().split()))


for i in range(n):
    temp.append( [arr[i], i+1-arr[i]])
    

temp = sorted(temp,key = lambda x:[x[0],-x[1]])

#print(temp)

seq = []


for [a,d] in temp:
    if d<0: continue 
    loc = bisect.bisect(seq,d)
    if loc==len(seq):  seq.append(d)
    else: seq[loc] = d


print(len(seq))
