# Non-strict fallback code rebinds the module globals compiled code is
# operating on: each hook drops the last reference to its receiver or to
# the other operand, then keeps using it; CPython's reflected operator
# and dict probe keep using theirs after the hook returns. Oracle: CPython
# runs this file unchanged (tests/object_operand_pin.rs).


class Left:
    def __init__(self, tag):
        self.tag = tag
        self.data = [tag] * 2

    def __eq__(self, other):
        global right
        right = None
        return NotImplemented

    def __add__(self, other):
        global right
        right = None
        return NotImplemented

    def __getattr__(self, name):
        global left
        left = None
        return name + " of " + self.tag + str(len(self.data))

    def method(self):
        global left
        left = None
        return "method of " + self.tag + str(len(self.data))

    def __len__(self):
        global left
        left = None
        return len(self.data)

    def __getitem__(self, index):
        global left
        left = None
        return self.data[index] + self.tag

    def __del__(self):
        print("del", self.tag)


class Right:
    def __init__(self, tag):
        self.tag = tag
        self.data = [tag] * 3

    def __eq__(self, other):
        return "Right.eq " + self.tag + str(len(self.data))

    def __radd__(self, other):
        return "Right.radd " + self.tag + str(len(self.data))

    def __del__(self):
        print("del", self.tag)


class Key:
    def __init__(self, tag):
        self.tag = tag

    def __hash__(self):
        global table
        table = None
        return 1

    def __eq__(self, other):
        return isinstance(other, Key)


left = Left("l1")
right = Right("r1")
print(left == right)
right = Right("r2")
print(left + right)
print(left.missing)
left = Left("l2")
print(left.method())
left = Left("l3")
print(len(left))
left = Left("l4")
print(left[0])
table = {Key("stored"): "value"}
print(table[Key("probe")])
table = {Key("stored"): "value"}
print(Key("probe") in table)
print(table)
