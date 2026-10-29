# v0.10 accept 2: compiled code reading, testing and storing into CPython
# dicts it receives (an exact dict through the dict API, a subclass through
# its own methods), and str literals crossing as CPython's interned strings.
# Driven from Python by `v10_exact_dicts_driver.py`.
def get(d, k):
    return d[k]


def has(d, k):
    return k in d


def put(d, k, v):
    d[k] = v
    return d


def get_raw(d):
    return d['raw']


def has_raw(d):
    return 'raw' in d


def put_raw(d, v):
    d['raw'] = v


def key_of(d):
    for k in d:
        return k


def literal():
    return 'raw'
