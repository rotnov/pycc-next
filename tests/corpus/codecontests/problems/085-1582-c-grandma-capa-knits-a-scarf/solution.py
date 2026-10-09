import sys
from collections import Counter
import math
import bisect
import heapq
#sys.stdin = open("input.txt")
from itertools import combinations
from itertools import accumulate
from collections import defaultdict
from collections import deque

num_cases = int(sys.stdin.readline().strip())

for case in range(1, num_cases+1):
    N = int(sys.stdin.readline().strip())
    S = list(str(sys.stdin.readline().strip()))

    type_alphabet = list(set(S))

    if len(type_alphabet) == 1:
        print(0)
    else:
        result = float('inf')

        for alphabet in type_alphabet:
            new_S = list(filter(lambda x: x != alphabet, S))
            break_signal = 0

            for i in range(len(new_S) // 2):
                if new_S[i] == new_S[len(new_S) - i -1]:
                    continue
                else:
                    break_signal = 1
                    break

            if break_signal == 0:
                index_basket = []
                semi_S = ['0'] + S + ['0']
                count = 0
                for index, value in enumerate(semi_S):
                    if value != alphabet:
                        index_basket.append(count)
                        count = 0
                    else:
                        count += 1

                index_basket = index_basket[1:]

                real_count = 0
                for i in range(len(index_basket) // 2):
                    must_erase = index_basket[i] + index_basket[len(index_basket) - 1 - i]
                    real_count += must_erase - min(index_basket[i], index_basket[len(index_basket)-1-i]) * 2



                result = min(result, real_count)

        if result == float('inf'):
            print(-1)
        else:
            print(result)
