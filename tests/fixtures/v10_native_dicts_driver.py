# Python code driving `v10_native_dicts.py` (imported as `m`), with a leak
# probe: repeated calls keep every object's reference count, and leave no
# native dict alive (the compiled module's live-dict counter; 0 for the
# source).
import ctypes
import sys

import m


class Holder:
    pass


def live_dicts():
    path = getattr(m, '__file__', '') or ''
    if not path.endswith('.so'):
        return 0
    return ctypes.CDLL(path).pycc_rt_dict_live_objects()


obj = object()
print(m.tally(['x', 'y', 'z']))
print(m.tally([]))
print(m.order('A', 'B'))
print(m.tests(obj))
print(m.missing([1]))
print(m.grow(3))
print(m.nested(obj))
print(m.rebound(False, 'v'), m.rebound(True, 'v'))

d = m.returned(obj)
print(type(d).__name__, d['a'] is obj, sorted(d))
seen = []
print(m.passed(obj, lambda d: (seen.append(d), d.__setitem__('a', 'changed'))), type(seen[0]).__name__)
h = Holder()
r = m.stored(h, obj)
print(h.d is r, sorted(r))
print(m.compared(1))
print(m.pickled('p'))
print(m.viewed(obj))

log = []


class Noisy:
    def __del__(self):
        log.append('del')


print(m.duplicated(Noisy, lambda: list(log)), log)


class Real(float):
    pass


print(m.narrowed(2.5), m.narrowed(Real(1.5)), m.narrowed('s'))

before = (sys.getrefcount(obj), live_dicts())
for _ in range(1000):
    m.tally([obj, obj])
    m.order(obj, obj)
    m.tests(obj)
    m.missing(obj)
    m.grow(obj)
    m.nested(obj)
    m.rebound(True, obj)
after = (sys.getrefcount(obj), live_dicts())
print('leaks', after[0] - before[0], after[1] - before[1])
