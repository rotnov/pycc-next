from itertools import combinations


def solve(days, all_students):
    required = all_students // 2
    for a, b in combinations(list(range(5)), 2):
        n1, n2 = len(days[a]), len(days[b])
        students = days[a].union(days[b])
        # print(students)
        if len(students) == all_students and n1 >= required and n2 >= required:
            return "YES"

    return "NO"


if __name__ == '__main__':
    for _ in range(int(input())):
        n = int(input())
        days = [set() for _ in range(5)]
        totals = [0] * 5
        for student in range(n):
            for day, value in enumerate(input().split()):
                if value == '1':
                    days[day].add(student)
                    totals[day] += 1

        print(solve(days, n))
