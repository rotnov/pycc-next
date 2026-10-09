def rotateRight(msg):
    if len(msg) == 1:
        return msg
    return msg[-1] + msg[:-1]

def rotateLeft(msg):
    if len(msg) == 1:
        return msg
    return msg[1:] + msg[0]

def swapHalf(msg):
    if len(msg) == 1:
        return msg
    H = len(msg) // 2
    if len(msg) % 2 == 0:
        return msg[H:] + msg[:H]
    else:
        return msg[H+1:] + msg[H] + msg[:H]

def increment(msg):
    arr = [ c for c in msg ]
    for i in range(len(arr)):
        if arr[i].isdigit():
            arr[i] = str((int(arr[i]) + 1) % 10)
    return "".join(arr)

def decrement(msg):
    arr = [ c for c in msg ]
    for i in range(len(arr)):
        if arr[i].isdigit():
            if arr[i] == '0':
                arr[i] = '9'
            else:
                arr[i] = str(int(arr[i]) - 1)
    return "".join(arr)

if __name__ == '__main__':
    N = int(input())

    for _ in range(N):
        order = input()
        msg = input()

        for i in range(len(order) - 1, -1, -1):
            c = order[i]
            if c == 'J':
                msg = rotateRight(msg)
            elif c == 'C':
                msg = rotateLeft(msg)
            elif c == 'E':
                msg = swapHalf(msg)
            elif c == 'A':
                msg = msg[::-1]
            elif c == 'P':
                msg = decrement(msg)
            elif c == 'M':
                msg = increment(msg)
        
        print(msg)
