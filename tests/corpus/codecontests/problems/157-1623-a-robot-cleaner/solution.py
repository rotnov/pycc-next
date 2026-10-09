import heapq
from collections import Counter
import math
import sys
input = sys.stdin.readline
############ ---- Input Functions ---- ############
def inp():
    return(int(input()))
def inlt():
    return(list(map(int,input().split())))
def insr():
    s = input()
    return(list(s[:len(s) - 1]))
def invr():
    return(map(int,input().split()))
def inis():
    return(input().split())
###################################################

# # Code to find top 3 elements and their counts
# # using most_common
#
# arr = [1, 3, 4, 1, 2, 1, 1, 3, 4, 3, 5, 1, 2, 5, 3, 4, 5]
# counter = Counter(arr)
# top_three = counter.most_common()
# print(sorted(top_three))
#
#
# # Python code to find 3 largest and 4 smallest
# # elements of a list.
#
# grades = [110, 25, 38, 49, 20, 95, 33, 87, 80, 90, 110]
# print(heapq.nlargest(3, grades))
# print(heapq.nsmallest(4, grades))

###########---------Code Here------------##############
for _ in range(inp()):
    a, b, c, d, e, f = invr()
    x = y = 1
    t = 0
    while c != e and d != f:
        if c != a:
            c += x
        else:
            x = -x
            c += x
        if d != b:
            d += y
        else:
            y = -y
            d += y
        t += 1
    print(t)
