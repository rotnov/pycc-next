def process(A):
    A = sorted(A)
    answer = -1*float('inf')
    S = sum(A)
    S1 = A[0]
    n1 = 1
    answer = max(answer, S1/n1+(S-S1)/(n-n1))
    for i in range(1, n-1):
        n1+=1
        S1+=A[i]
        answer = max(answer, S1/n1+(S-S1)/(n-n1))
    return answer

t = int(input())
for i in range(t):
    n = int(input())
    A = [int(x) for x in input().split()]
    print(process(A))
