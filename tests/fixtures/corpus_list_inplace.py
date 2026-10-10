a = [1, 2]
alias = a
a += [3]
print(a, alias)
a *= 2
print(alias)
a *= 0
print(alias, a)
b = [1]
b = b + [2]
print(b)


class Box:
    items: list[str]

    def __init__(self) -> None:
        self.items = ["x"]


box = Box()
keep = box.items
box.items += ["y"]
print(keep)
grid = [[0], [1]]
row = grid[0]
grid[0] += [5]
print(row, grid)
acc: list[int] = []
for i in range(4):
    acc += [i * i]
    acc = acc + [0]
print(acc)
acc += acc
print(len(acc))
w = [True]
w *= 3
print(w)
