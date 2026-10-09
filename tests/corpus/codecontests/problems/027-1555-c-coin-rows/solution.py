#!/usr/bin/env python
# coding: utf-8

# In[25]:


T = int(input())
out = []
for t in range(T):
    m = int(input())
    a1 = [int(x) for x in input().split()]
    a2 = [int(x) for x in input().split()]
    pre1 , pre2 = [a1[0]] , [a2[0]]
    if m==1:
        out.append(0)
        continue
    suf1  = [a1[-1]]
    for i in range(1,m):
        pre2.append(pre2[-1]+a2[i])
    for i in range(m-2,-1,-1):
        suf1.append(suf1[-1]+a1[i])
    
    suf1 = suf1[::-1]
    bob = float('inf')
    opt1 = 0
    for i in range(m):
        if i < m-1 and i > 0: 
            ans = max(suf1[i+1] , pre2[i-1])
        elif i < m-1:
            ans = suf1[i+1]
        else:
            ans = pre2[i-1]
        if  ans < bob: 
            opt1=i
            bob = ans
            #print(bob)
           
    out.append(bob)
for e in out:
    print(e)
        


# In[ ]:
