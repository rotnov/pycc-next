# v0.10 accept 2: `d[key]` with a CPython-object key is not admitted on a
# native `str`-keyed dict, so `lookup`'s display stays a CPython dict
# (`pycc_types::non_strict::object_dicts`) while `local`'s is native; both
# run as CPython runs them.


def lookup(key, obj):
    d = {'a': obj, 'b': 2}
    try:
        return d[key]
    except KeyError as e:
        return 'KeyError ' + str(e)


def local(obj):
    d = {'x': obj, 'y': None}
    return str(len(d)) + ' ' + str(d['y'])


for k in ['a', 'b', 'z', 5, None, (1, 2)]:
    print(repr(k), lookup(k, [k]))
print(local(1))
