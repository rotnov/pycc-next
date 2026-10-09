import math
#s = input()
#n= (map(int, input().split()))

#(map(int, input().split()))


#a, b = (map(int, input().split()))

for i in range(0, int(input())):

    n, h =(map(int, input().split()))
    a = list(map(int, input().split()))

    dist = list()

    for j in range(0, len(a)-1):
        dist.append(a[j+1]-a[j])

    dist.sort()

    if(len(dist)==0):
        dist.append(a[0])
    else:
        dist.append(dist[len(dist)-1])
    sum = 0
    pred_sum = 0
    flag = 0
    k = 0
    for j in range(0, len(a)):
        sum = pred_sum+(len(a)-j)*dist[j]
        if(sum>=h):
            flag = 1
            k = j
            break

        pred_sum += dist[j]

    if(flag == 0):
        if(len(dist)==1):
            print(h)
        else:
            print(h-pred_sum+dist[len(dist)-1])
    else:
        sum = 0
        sum = (h-pred_sum)//(len(a)-k)
        if((h-pred_sum)%(len(a)-k)):
            sum += 1

        print(sum)
