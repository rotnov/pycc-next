def solution(n, k, x, s):
    current_len = 0
    astrics = []
    for c in s:
        if   c == '*':
            current_len += 1
        elif c == 'a':
            if current_len != 0:
                astrics.append(current_len)
            current_len = 0
    if current_len != 0:
        astrics.append(current_len)
    astrics = list(map(lambda y: k*y+1, astrics))
    x -= 1
    result = []
    for num in reversed(astrics):
        m = x % num
        x //= num
        result.append(m)

    answer = ''
    c = s[0]
    if c == 'a':
        answer = 'a'
    for i in range(1, n):
        if s[i] == 'a' and s[i - 1] == '*':
            num = result.pop()
            answer += num * 'b'
        if s[i] == 'a':
            answer += 'a'
    if s[n - 1] == '*':
        num = result.pop()
        answer += num * 'b'


    return answer


if __name__ == '__main__':
    t = int(input())
    for i in range(t):
        n, k, x = list(map(int, input().split()))
        s = input()
        print(solution(n, k, x, s))
