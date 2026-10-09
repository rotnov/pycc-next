# -*- coding: UTF-8 -*-
t = int(input())
inputdata = []
for i in range(t):
    input()
    inputdata.append(input().split(" "))
    for j in range(len(inputdata[i])):
        inputdata[i][j] = int(inputdata[i][j])
for i in range(t):
    res = 0
    l = len(inputdata[i])
    ost = sum(inputdata[i]) % l
    while(ost > 0):
        ost -= l;
        res += 1
    print(res)
        
    
