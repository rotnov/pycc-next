
import sys
input = sys.stdin.buffer.readline
 
############ ---- Input Functions ---- ############
def inp(): 
    # one integer
    return int(input())
def input_gen_list():
    return input()[:-1].split()
def input_list():
    # list of integers
    return list(map(int,input().split()))
def input_string():
    # list of characters
    s = input()
    return list(s[:len(s) - 1])
def input_int_gen():
    # integer generator 
    return map(int,input().split())
 
# Codeforce 747 Div 2 E
from collections import defaultdict
MOD = 10 ** 9 + 7
tests = 1
color = {"white": 1, "yellow": 6, "green": 2, "blue": 5, "red": 3, "orange" : 4}
counts = defaultdict(lambda:None)
mands = dict()
marks = set()
marks_ls = []

def powerm(a, b): 
    # calculate a ** b % MOD
    prodi = 1
    base = a
    while b:
        if b % 2 == 1:
            prodi = (prodi * base) % MOD
        base = (base * base) % MOD
        b = b >> 1
    return prodi

def under(node, k):
    # count number ways to color nodes under node excluding node, k: total height
    num_nodes = 2 ** (k - node.bit_length() + 1) - 2
    return powerm(4, num_nodes)

def mark_up(node):
    while node:
       marks.add(node)
       marks_ls.append(node)
       node = node // 2
def par_val(node1, node2):
    if node1 == node2 == None:
        return [1 for _ in range(7)]
    if node1 == None or node2 == None:
        node = node1 if node2 == None else node2
        ret = [0 for _ in range(7)]
        for x in range(1, 7):
            for y in range(1, 7):
                if x == y or x + y == 7:
                    continue
                ret[x] = (ret[x] + (node[y]) % MOD) % MOD
        return ret
    ret = [0 for _ in range(7)]
    for x in range(1, 7): # par
        for y in range(1, 7): # node1
            if x == y or x + y == 7:
                continue
            for z in range(1, 7): #node2
                if x == z or x + z == 7:
                    continue
                ret[x] = (ret[x] + (node1[y] * node2[z]) % MOD) % MOD
    return ret
for _ in range(tests):
    k = inp()
    n = inp()
    for _ in range(n):
        l = input_gen_list()
        v, c = int(l[0]), color[l[1].decode()]
        mands[v] = c
        mark_up(v)
    marks_ls.sort(reverse=True)
    # print(mands, mark_up, marks_ls)
    for node in marks_ls:
        child1, child2 = counts[node * 2], counts[node * 2 + 1]
        cur = par_val(child1, child2)
        if node in mands:
            counts[node] = [0 if i != mands[node] else x for i, x in enumerate(cur)]
        else:
            counts[node] = cur
    # print(counts[4], counts[5])
    # print(par_val(counts[4], counts[5]))
    print(sum(counts[1]) * powerm(4, 2 ** k - 1 - len(marks)) % MOD)
    
