import sys
input = sys.stdin.readline

T = int(input())
result = []

for _ in range(T):
    raw_keyboard = input()[:-1]
    word = input()[:-1]
    t = 0

    if len(word) == 1:
        result.append(str(0))
        continue

    key_map = {}
    for i, k in enumerate(raw_keyboard):
        key_map[k] = i + 1

    for i in range(len(word)-1):
        t += abs(key_map[word[i]] - key_map[word[i+1]])
    result.append(str(t))

print('\n'.join(result))
