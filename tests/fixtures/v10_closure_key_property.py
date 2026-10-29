# v0.10: `sorted` keys a copy of its whole input, so a key whose attribute
# read runs a property getter that changes the input list does not change
# what is sorted.


class Holder:
    def __init__(self, xs):
        self.xs = xs

    @property
    def value(self):
        self.xs[1] = 0
        return 0


def by_property(xs):
    holder = Holder(xs)

    def key(v):
        return holder.value

    return sorted(xs, key=key), xs


print(by_property([2, 1]))
