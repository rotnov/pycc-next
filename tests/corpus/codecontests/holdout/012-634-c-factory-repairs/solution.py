from functools import reduce
class SegmentTree():
    def __init__(self, L, function = lambda x,y: x+y, initilizer = None):
        self.function = function
        self.initilizer = initilizer
        N = self.size = len(L)
        M = 1 << N.bit_length()
        self.margin = 2*M - N
        self.L = [None for i in range(self.margin)] + L
        for i in range(M-1, 0, -1):
            x, y = self.L[i<<1], self.L[i<<1|1]
            self.L[i] = None if x is None or y is None else function(x, y)
    def modify(self, pos, value):
        p = pos + self.margin
        self.L[p] = value 
        while p > 1:
            x, y = self.L[p], self.L[p^1]
            if p&1: x, y = y, x
            self.L[p>>1] = None if x is None or y is None else self.function(x, y)
            p>>=1
    def query(self, left, right):
        l, r = left + self.margin, right + self.margin
        stack = []
        void = True
        if self.initilizer is not None:
            void = False
            result = self.initilizer
        while l < r:
            if l&1:
                if void:
                    result = self.L[l]
                    void = False
                else:
                    result = self.function(result, self.L[l])
                l+=1
            if r&1:
                r-=1
                stack.append(self.L[r])
            l>>=1
            r>>=1
        init = stack.pop() if void else result
        return reduce(self.function, reversed(stack), init)

import sys
n, k, a, b, q = [int(x) for x in input().split()]
orders = [0]*(n+2)
a_tree, b_tree = SegmentTree(orders, initilizer = 0), SegmentTree(orders, initilizer = 0)
for line in sys.stdin:
    s = [int(x) for x in line.split()]
    if s[0] == 1:
        orders[s[1]] += s[2]
        a_tree.modify(s[1], min(a, orders[s[1]]))
        b_tree.modify(s[1], min(b, orders[s[1]]))
    else:
        query = b_tree.query(0, s[1]) + a_tree.query(s[1]+k, n+1)
        print(query)
