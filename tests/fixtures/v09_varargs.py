# v0.9 accept 1: `*args` and `**kwargs` of CPython objects in a native
# class -- a constructor's `*items`, a static method's `*context, **kwargs`,
# a module function's -- bound as CPython binds them, from compiled code and
# from Python code alike.


class Stack:
    def __init__(self, *items):
        """The stack, bottom first."""
        self._stack = list(items)
        self.kind = type(items).__name__

    def __repr__(self):
        return "%s%s" % (self.__class__.__name__, tuple(self._stack))

    @staticmethod
    def create(*context, **kwargs):
        items = context
        context = Stack()
        for item in items:
            if item is None:
                continue
            context.push(item)
        if kwargs:
            context.push(kwargs)
        return context

    def push(self, item):
        self._stack.append(item)

    def copy(self):
        return Stack(*self._stack)

    def top(self):
        return self._stack[-1]


class Pair:
    def __init__(self, first, second, *rest):
        self.first = first
        self.second = second
        self.rest = rest

    def describe(self):
        return [self.first, self.second, self.rest, type(self).__name__]


class Triple:
    def __init__(self, a, b, c, *more):
        self.total = a + b + c + sum(more)


class _Registry:
    @staticmethod
    def build(*names, **options):
        return {"names": names, "options": options, "fresh": {"n": len(names)}}


def gather(*values, **named):
    return [values, sorted(named.items())]


def run():
    s = Stack.create({"a": 1}, None, {"b": 2}, c=3)
    t = s.copy()
    print(s.kind, t.kind, t.top(), len(t._stack), repr(t))
    print(Pair(1, 2).describe(), Pair(1, 2, 3, [4]).describe())
    print(_Registry.build("x", "y", strict=True))
    print(gather(), gather(1, "a", k=None))
    print(sorted(vars(Pair("p", "q")).items()), Stack(5).__dict__ == {"_stack": [5], "kind": "tuple"})


run()
