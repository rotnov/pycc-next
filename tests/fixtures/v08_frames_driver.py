"""Drives `v08_frames_ext.py` (compiled, or its source): `python3 driver.py DIR`."""
import os
import sys
import traceback
import warnings

sys.path.insert(0, sys.argv[1])
import v08_frames_ext as m  # noqa: E402


def show_tb(exc):
    for frame in traceback.extract_tb(exc.__traceback__):
        print(" ", os.path.basename(frame.filename), frame.lineno, frame.name, repr(frame.line))


def via_python(n):
    return m.depth_name(n)


print("import", m.AT_IMPORT)
for n in range(3):
    print("depth", n, m.outer(n))


def asks_info():
    return m.caller_info(1)


print("info", asks_info()[:2], asks_info()[3:])
print("info line", m.caller_info(0)[2])
print("through", [m.through_python(via_python, n) for n in range(4)])
print("box", m.Box(1).where())
print("stack", m.stack_names())

try:
    m.call_boom(0)
except ZeroDivisionError as e:
    print("boom", e)
    show_tb(e)
try:
    m.reraises([1, 2], 5)
except ValueError as e:
    print("reraises", e)
    show_tb(e)
try:
    m.Box(3).fail()
except ZeroDivisionError as e:
    show_tb(e)


def flaky(i):
    if i == 2:
        raise KeyError(i)
    return i


try:
    m.calls_back(flaky)
except KeyError as e:
    show_tb(e)

print("handled", m.handled_traceback(0))

with warnings.catch_warnings(record=True) as caught:
    warnings.simplefilter("always")
    m.warns(1)
print("warn", [(os.path.basename(w.filename), w.lineno, str(w.message)) for w in caught])

print("version", m.version_short())
m.VERSION = "3.1.4"
print("version", m.version_short())
m.LIMIT = 10
print("limit", m.limit_plus(1))
m.deepcopy = lambda x: ("patched", x)
print("copies", m.copies([1]))
print("private", m._private_twice(21))


def calls_defaulted():
    return m.defaulted(), m.defaulted(2, scale=3), m._caller_name(1)


print("defaulted", calls_defaulted())
try:
    m.defaulted(-1)
except ValueError as e:
    show_tb(e)
try:
    m.defaulted(1, 2)
except TypeError as e:
    print("bind", e)
    show_tb(e)

def asks_lifted():
    box = m.Lifted(4)
    return box.where(), box.where(n=0), box.where(2), box.prop, m.Lifted.make(5)


print("lifted", asks_lifted())
box = m.Lifted(6)
print("lifted bind", box.bind(1), box.bind(1, 5, 6, 7, c=8, z=9), box.bind(a=4, b=0))
print("lifted call", m.Lifted.bind(box, *[1, 2, 3], **{"c": 0}), m.Lifted.where(box, 0))
for bad in ((), (1,), (1, 2)):
    try:
        if bad == ():
            box.bind()
        elif bad == (1,):
            box.bind(1, a=2)
        else:
            box.where(1, n=2)
    except TypeError as e:
        print("lifted error", e)
        show_tb(e)
for x in (0, 0):
    try:
        box.fails(x)
    except ZeroDivisionError as e:
        show_tb(e)

# Constant memory: frames and tracebacks are released.
import resource  # noqa: E402


def churn(count):
    for i in range(count):
        try:
            m.call_boom(0)
        except ZeroDivisionError:
            pass
        m.outer(1)
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss


small = churn(20000)
large = churn(200000)
print("memory", "constant" if large - small < 4000 else f"grew {large - small} kB")
