# v0.10 accept 2: a function's `str`-keyed dict display holding CPython
# objects is a native dict when it never escapes the function, and the
# CPython dict it was when it may. Driven from Python by
# `v10_native_dicts_driver.py`.
import pickle


def tally(words):
    seen = {'first': None, 'last': None, 'n': 0}
    for w in words:
        if seen['first'] is None:
            seen['first'] = w
        seen['last'] = w
        seen['n'] = seen['n'] + 1
    out = []
    for k in seen:
        out.append(k + '=' + str(seen[k]))
    return ','.join(out)


def order(a, b):
    # Overwriting a key keeps its place; a new key goes last.
    d = {'z': a, 'y': b, 'x': 1}
    d['y'] = a
    d['w'] = b
    d['z'] = 3.5
    keys = []
    for k in d:
        keys.append(k)
    return ' '.join(keys) + ' ' + str(d['z']) + ' ' + str(d['y'] is a)


def tests(obj):
    d = {'a': obj, 'b': None}
    out = []
    if d:
        out.append('truthy')
    out.append(str('a' in d))
    out.append(str('c' in d))
    out.append(str('c' not in d))
    out.append(str(len(d)))
    out.append(str(d['b'] is None))
    out.append(str(d['a'] is obj))
    return ' '.join(out)


def missing(obj):
    d = {'a': obj}
    try:
        return d['b']
    except KeyError as e:
        return 'KeyError ' + str(e)


def grow(obj):
    d = {'a': obj, 'b': obj}
    try:
        for k in d:
            d[k + '!'] = obj
    except RuntimeError as e:
        return str(e)
    return 'no error'


def nested(obj):
    d = {'type': 'text', 'raw': obj, 'children': [], 'attrs': {'level': 1}}
    return str(d['children']) + ' ' + str(d['attrs']) + ' ' + str(d['raw'] is obj)


def rebound(flag, obj):
    d = {'k': obj}
    if flag:
        d = {'k': 'other', 'j': obj}
    return str(len(d)) + ' ' + str(d['k'])


# Each of these may escape, so it is CPython's dict: the caller sees the
# object itself.
def returned(obj):
    d = {'a': obj, 'b': 1}
    return d


def passed(obj, f):
    d = {'a': obj}
    f(d)
    return d['a']


def stored(holder, obj):
    d = {'a': obj}
    holder.d = d
    d['b'] = 2
    return d


def compared(obj):
    d = {'a': obj, 'b': 2}
    e = {'b': 2, 'a': obj}
    return d == e, repr(d)


def pickled(obj):
    d = {'a': obj, 'b': [1, 2]}
    return pickle.loads(pickle.dumps(d)) == d


def viewed(obj):
    d = {'a': obj}
    keys = d.keys()
    d['b'] = 2
    return list(keys)
