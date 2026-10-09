def input_ints():
    return list(map(int, input().split()))

def output_list(v):
    print(' '.join(str(x) for x in v))

def main():
    t = int(input())
    for _ in range(t):
        a, b, n = input_ints()
        print([a, b, a ^ b][n % 3])


if __name__ == '__main__':
    main()
