# A module's globals read as attributes of the module object, through the
# attribute cache, and rebound and deleted between the reads.
import types


def read_many(me, n):
    total = 0
    for _ in range(n):
        total += len(me.LIMIT) + me.COUNT
    return total


def main():
    me = types.ModuleType("config")
    exec("LIMIT = 'strict'\nCOUNT = 5\n", me.__dict__)
    print(read_many(me, 200))
    me.LIMIT = "xx"
    print(read_many(me, 3))
    del me.COUNT
    try:
        read_many(me, 1)
    except AttributeError as e:
        print("AttributeError", e.name)


if __name__ == "__main__":
    main()
