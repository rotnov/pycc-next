import os, sys, types, hashlib, inspect, dataclasses, collections
def shape(c):
    if isinstance(c, types.CodeType):
        return (c.co_code, tuple(shape(k) for k in c.co_consts), c.co_names, c.co_varnames, c.co_freevars,
                c.co_cellvars, c.co_linetable, c.co_exceptiontable, c.co_flags, c.co_qualname, c.co_name,
                c.co_firstlineno, c.co_stacksize, c.co_argcount, c.co_posonlyargcount, c.co_kwonlyargcount,
                os.path.basename(c.co_filename))
    if isinstance(c, (tuple, frozenset)):
        return (type(c).__name__, tuple(shape(k) for k in c))
    return repr(c)
def digest(c):
    return hashlib.sha1(repr(shape(c)).encode()).hexdigest()[:12]
events = []
def audit(e, a):
    if e == "compile" and a[1] == "<string>":
        events.append(("compile", hashlib.sha1(a[0]).hexdigest()[:12]))
    elif e == "exec" and isinstance(a[0], types.CodeType) and a[0].co_filename == "<string>":
        events.append(("exec", a[0].co_name))
sys.addaudithook(audit)
mode = sys.argv[1] if len(sys.argv) > 1 else ""
given = []
if mode == "patched":
    import builtins
    real = builtins.exec
    def patched(source, *args, **kwargs):
        if sys._getframe(1).f_code.co_name == "add_fns_to_class":
            given.append(type(source).__name__)
        return real(source, *args, **kwargs)
    builtins.exec = patched
elif mode == "bound":
    dataclasses.exec = exec
elif mode == "static":
    # A builtin with no `self` bound as `exec`.
    dataclasses.exec = str.maketrans
elif mode == "eval":
    # Another builtin of `builtins` replacing `exec`.
    import builtins
    builtins.exec = builtins.eval
used = collections.Counter()
def prof(frame, ev, arg):
    if ev == "c_call" and frame.f_code.co_name == "add_fns_to_class" and getattr(arg, "__name__", "") == "exec":
        tables = getattr(arg, "__self__", None)
        ahead = type(tables) is list and any(frame.f_locals["txt"] in t for t in tables)
        used["ahead" if ahead else "text"] += 1
sys.setprofile(prof)
if mode in ("static", "eval"):
    try:
        import m
    except Exception as e:
        print("import:", type(e).__name__, e)
    print("ahead-of-time:", used["ahead"], "from text:", used["text"], file=sys.stderr)
    sys.exit()
if mode == "threads":
    # Two chunks running at once, the first ending first.
    import threading, sync
    threading.setprofile(prof)
    def control():
        sync.ev["a in"].wait(30)
        other = threading.Thread(target=__import__, args=("tb",))
        other.start()
        sync.ev["b in"].wait(30)
        sync.ev["a go"].set()
        sync.ev["a done"].wait(30)
        sync.ev["b go"].set()
        other.join()
    controller = threading.Thread(target=control)
    controller.start()
    import ta
    sync.ev["a done"].set()
    controller.join()
    sys.setprofile(None)
    import tb
    print("ahead-of-time:", used["ahead"], "from text:", used["text"], file=sys.stderr)
    print(events)
    print("exec bound:", "exec" in vars(dataclasses))
    print(ta.A(), tb.B(), ta.A(2) == ta.A(2), repr(tb.B("c")))
    sys.exit()
import m, fut
sys.setprofile(None)
print("ahead-of-time:", used["ahead"], "from text:", used["text"], file=sys.stderr)
print(events)
print("exec bound:", "exec" in vars(dataclasses), vars(dataclasses).get("exec") is exec, sorted(set(given)))
def show(cls):
    print("==", cls.__module__, cls.__qualname__, cls.__doc__, getattr(cls, "__slots__", None),
          getattr(cls, "__match_args__", None), [f.name for f in dataclasses.fields(cls)])
    print("   ", inspect.signature(cls), cls.__dataclass_params__)
    for name in ("__init__", "__repr__", "__eq__", "__lt__", "__le__", "__gt__", "__ge__", "__hash__",
                 "__setattr__", "__delattr__", "__getstate__", "__setstate__"):
        fn = cls.__dict__.get(name)
        if isinstance(fn, types.FunctionType):
            c = fn.__code__
            print("   ", name, fn.__qualname__, fn.__module__, fn.__defaults__, fn.__kwdefaults__,
                  os.path.basename(c.co_filename), c.co_firstlineno, c.co_varnames, c.co_freevars,
                  digest(c) if c.co_filename == "<string>" else "-", sorted(vars(fn)),
                  getattr(fn, "__wrapped__", None) is not None)
            if hasattr(fn, "__wrapped__"):
                w = fn.__wrapped__.__code__
                print("    wrapped", fn.__wrapped__.__qualname__, os.path.basename(w.co_filename), w.co_firstlineno, digest(w))
            if hasattr(fn, "__annotate__") and fn.__annotate__ is not None:
                print("    annotations", inspect.get_annotations(fn))
        else:
            print("   ", name, type(fn).__name__)
for mod in (m, fut):
    for k, v in vars(mod).items():
        if isinstance(v, type) and dataclasses.is_dataclass(v) and v.__module__ == mod.__name__:
            show(v)
p = m.Plain(1); print(p, p == m.Plain(1, "y", []), m.Plain(2, z=[3]))
f = m.FrozenSlots(1); print(f, hash(f) == hash(m.FrozenSlots(1)), f.norm())
try:
    f.a = 5
except dataclasses.FrozenInstanceError as e:
    print("frozen:", e)
o1, o2 = m.Ordered(1, 9.0), m.Ordered(2); print(o1 < o2, o1 == m.Ordered(1, 3.0), hash(o1) == hash(m.Ordered(1, 3.0)), repr(o1)[:20])
k = m.KwOnly(p=1); print(k == m.KwOnly(p=1), k.q)
try:
    m.KwOnly(1)
except TypeError as e:
    print("kw:", e)
s = m.Specials(2, "ab", flag=True); print(s, s.hidden, m.Specials.count)
print(m.Child(1, extra=3), m.FrozenChild(4), m.OwnEq(1) == m.OwnEq(2), m.OwnEq.__hash__)
print(m.Twin1.__init__.__code__ is m.Twin2.__init__.__code__, m.Twin1(1), m.Twin2(2))
print(m.FromElsewhere(2, 3), m.Weak(1))
d = fut.Deferred(1, d="abc"); print(d, fut.Later(d))
try:
    m.Plain()
except TypeError as e:
    print("missing:", e)
