# v0.9 accept 1: a list or dict slot of a class that holds CPython objects is
# a CPython list or dict (the class stays native), and a private class's
# methods answer CPython code that reaches one of its instances.


class Stack:
    def __init__(self):
        self._stack = []
        self._index = {}

    def push(self, item):
        self._stack.append(item)

    def put(self, key, item):
        self._index[key] = item

    def lookup(self, key):
        return self._index[key]

    def pop(self):
        return self._stack.pop()

    def top(self):
        return self._stack[-1]

    def size(self):
        return len(self._stack)


class _Node:
    def __init__(self, key):
        self.key = key

    def render(self, stack):
        return self.key + "=" + str(stack.top())

    def _hidden(self, stack, suffix):
        return str(stack.size()) + suffix


def make_nodes(keys):
    return [_Node(k) for k in keys]


def run(x):
    s = Stack()
    s.push(x)
    s.push({"a": 1})
    s.put("k", [x, x])
    first = s.top()
    popped = s.pop()
    return first, popped, s.top(), s.lookup("k"), s.size()


print(run(3))
print(run("text"))
s = Stack()
s.push(1)
# The list is the instance's own: a store through it is seen by the class.
s._stack.append([2])
print(s.top(), s.size(), type(s._stack).__name__)
s._stack = [7, 8]
print(s.top(), s.size())
nodes = make_nodes(["a", "b"])
print([type(n).__name__ for n in nodes])
print([n.render(s) for n in nodes])
print(nodes[0]._hidden(s, "!"))
print(type(nodes[0]).__qualname__, sorted(a for a in dir(nodes[0]) if not a.startswith("__")))
# `__dict__` / `vars()` of an instance: its assigned attributes.
print(vars(nodes[1]), sorted(s.__dict__))
try:
    s.lookup("missing")
except KeyError as e:
    print("KeyError", e)
s.pop()
s.pop()
s.top()
