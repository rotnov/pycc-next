def materialize(n: int, step: int) -> None:
    try:
        print(len(list(range(n))))
    except OverflowError as e:
        print("list", "OverflowError", e)
    try:
        print(len(tuple(range(0, n, step))))
    except OverflowError as e:
        print("tuple", "OverflowError", e)
    try:
        print(len(sorted(range(n, 0, -step))))
    except OverflowError as e:
        print("sorted", "OverflowError", e)
    try:
        print(list(range(n, n + 3)), list(range(n + 3, n, -1)), list(range(n, n)))
    except OverflowError as e:
        print("small", "OverflowError", e)
    try:
        print(len(list(range(-n, n, n // 4))))
    except OverflowError as e:
        print("wide", "OverflowError", e)
    try:
        print(list(range(0, n, 0)))
    except ValueError as e:
        print("zero", "ValueError", e)


materialize(10 ** 30, 1)
materialize(10, 3)
materialize(2 ** 63, 2 ** 62)
print(list(range(2 ** 63 + 5, 2 ** 63 + 1, -2)), [i for i in range(3)])
print(len(list(range(10 ** 30))))
