import sys
input = sys.stdin.readline
write = sys.stdout.write

T = int(input())
result = []
for _ in range(T):
    keyboard = input()[:-1]
    word = input()[:-1]
    key_map = {}
    for i, k in enumerate(keyboard):
        key_map[k] = i + 1
    t = 0
    for i in range(len(word) - 1):
        t += abs(key_map[word[i]] - key_map[word[i + 1]])
    result.append(str(t))
write("\n".join(result) + "\n")
rest = sys.stdin.read()
print(len(rest), rest.split())
