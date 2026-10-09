def main():
    l = input().split()
    print(*l)
    for _ in range(int(input())):
        a, b = input().split()
        l[a == l[1]] = b
        print(*l)


if __name__ == '__main__':
    main()
