# v0.10: a `map`/`filter` that is not consumed at once stays lazy: its
# function reads captured state as it is when each item is consumed.


def lazy(values):
    state = [1]

    def shift(v):
        return v + state[0]

    def keep(v):
        return v > state[0]

    mapped = map(shift, values)
    kept = filter(keep, values)
    state[0] = 10
    return list(mapped), list(kept), list(map(shift, values))


print(lazy([1, 2, 11, 12]))
