# -*- coding: utf-8 -*-
"""
http://judge.u-aizu.ac.jp/onlinejudge/description.jsp?id=0206

"""
import sys
from sys import stdin
input = stdin.readline


def main(args):
    while True:
        L = int(input())
        if L == 0:
            break
        ans = 'NA'
        for i in range(1, 12+1):
            if ans == 'NA':
                M, N = map(int, input().split())
                L -= (M - N)
                if L <= 0:
                    ans = i
            else:
                _ = input()
        print(ans)


if __name__ == '__main__':
    main(sys.argv[1:])
