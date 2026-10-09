import sys
#sys.setrecursionlimit(20000)
#from collections import deque #Counter
#from itertools import accumulate
#from functools import product
#import math


def rall():
    return sys.stdin.readlines()
def rl():
    return sys.stdin.readline().strip()
def rl_types(types):
    str_list = [x for x in sys.stdin.readline().strip().split(' ')]
    return [types[i](str_list[i]) for i in range(len(str_list))]

def pr( something='' ):
    sys.stdout.write( str(something) + '\n')
def pra( array ):
    sys.stdout.write( ' '.join([str(x) for x in array]) + '\n')


def solve(array):
    return array


if __name__ == '__main__':

    NT = int( rl() )
    #a,b = map(int,rl().split(' '))

    for ti in range(NT):
        n = int(rl())
        array = list(map(int, rl().split(' ')))
        colors = rl()# = list(map(int, rl().split(' ')))
        blues = sorted([array[i] for i in range(n) if colors[i]=='B'])
        reds = sorted([array[i] for i in range(n) if colors[i]=='R'],reverse=True)
        #print('reds:',reds)
        #print('blues:',blues)
        reds_okay,blues_okay = True,True
        for i in range(len(blues)):
            if blues[i] >= 1+i:
                continue
            else:
                blues_okay = False
                break
        for i in range(len(reds)):
            if reds[i] <= n-i:
                continue
            else:
                reds_okay = False

        #a,b = map(int, rl().split(' '))
        # vals = rl_types( [str,float,float] )
        #pr(colors)
        #pr(array)
        pr('YES' if blues_okay and reds_okay else 'NO')
