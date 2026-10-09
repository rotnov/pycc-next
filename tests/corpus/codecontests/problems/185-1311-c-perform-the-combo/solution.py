for _ in range(int(input())):
    n, m = map(int, input().split())
    s = input()
    nums = [int(x) for x in input().split(" ")]

    answer = [0] * 26
    for ch in s:
        answer[ord(ch) - 97] += 1

    nums.sort()

    for i in range(m):
        if i == 0:
            for j in range(0, nums[0]):
                answer[ord(s[j]) - 97] += m - i
        else:
            for j in range(nums[i-1], nums[i]):
                answer[ord(s[j]) - 97] += m - i

    for num in answer:
        print(num, end=" ")
    print("\n")
