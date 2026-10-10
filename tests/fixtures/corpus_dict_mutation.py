def grow(d: dict[int, int]) -> None:
    try:
        for k in d:
            d[k + 100] = k
    except RuntimeError as e:
        print("RuntimeError", e)


def shrink(d: dict[int, int]) -> None:
    try:
        for k in d:
            del d[k]
    except RuntimeError as e:
        print("RuntimeError", e)


def update_values(d: dict[int, int]) -> None:
    for k in d:
        d[k] = d[k] * 10
    print(d)


def delete_then_break(d: dict[int, int]) -> None:
    for k in d:
        del d[k]
        break
    print(d)


def set_grow(s: set[int]) -> None:
    try:
        for x in s:
            s.add(x + 100)
    except RuntimeError as e:
        print("RuntimeError", e)


grow({1: 1, 2: 2})
shrink({1: 1, 2: 2})
shrink({7: 7})
update_values({1: 1, 2: 2})
delete_then_break({1: 1, 2: 2})
set_grow({1, 2})
d = {1: 1}
for k in d:
    d[2] = 2
