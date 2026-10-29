# Python code driving `v10_exact_dicts.py` (imported as `m`).
import collections

from m import get, has, put, get_raw, has_raw, put_raw, key_of, literal


def show(f, *args):
    try:
        print(repr(f(*args)))
    except Exception as e:
        print(type(e).__name__, repr(e.args))


class Missing(dict):
    def __missing__(self, key):
        return 'missing:' + str(key)


class Loud(dict):
    def __getitem__(self, key):
        return 'loud:' + str(key)

    def __contains__(self, key):
        return key == 'only'

    def __setitem__(self, key, value):
        dict.__setitem__(self, key, value * 2)


plain = {'raw': 'text', 'type': 'paragraph', (1, 2): 'pair', 3: 'three'}
show(get, plain, 'raw')
show(get, plain, (1, 2))
show(get, plain, 3)
show(get, plain, 'absent')
show(get, plain, (4, 5))
show(get, plain, [1])
show(has, plain, 'type')
show(has, plain, 'nope')
show(has, plain, [1])
show(put, {}, 'k', 1)
show(put, {}, [1], 1)
show(get_raw, plain)
show(get_raw, {})
show(has_raw, plain)
show(has_raw, {'x': 1})
d = {}
put_raw(d, 5)
print(d)
show(get, Missing(a=1), 'a')
show(get, Missing(a=1), 'b')
show(get_raw, Missing())
loud = Loud()
show(get, loud, 'x')
show(has, loud, 'only')
show(has, loud, 'x')
put_raw(loud, 21)
print(dict(loud))
od = collections.OrderedDict(raw=1)
show(get_raw, od)
show(has_raw, od)
dd = collections.defaultdict(list)
show(get_raw, dd)
print(dict(dd))
show(get, {}, 'k')
# A literal crosses as CPython's interned string, as a code constant does.
RAW = 'raw'
print(literal() is key_of({'raw': 0}))
print(key_of({literal(): 0}) is RAW)
print(literal() is ''.join(['r', 'aw']))
