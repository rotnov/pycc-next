import os, sys, types, hashlib, collections, typing
def shape(c):
    if isinstance(c, types.CodeType):
        return (c.co_code, tuple(shape(k) for k in c.co_consts), c.co_names, c.co_varnames, c.co_freevars,
                c.co_cellvars, c.co_linetable, c.co_exceptiontable, c.co_flags, c.co_qualname, c.co_name,
                c.co_firstlineno, c.co_stacksize, c.co_argcount, c.co_posonlyargcount, c.co_kwonlyargcount,
                c.co_filename)
    if isinstance(c, (tuple, frozenset)):
        return (type(c).__name__, tuple(shape(k) for k in c))
    return repr(c)
def digest(c):
    return hashlib.sha1(repr(shape(c)).encode()).hexdigest()[:12]
events = []
def audit(e, a):
    if e == "compile" and a[1] == "<string>":
        events.append(("compile", a[0].decode() if isinstance(a[0], bytes) else type(a[0]).__name__))
sys.addaudithook(audit)
import importlib
owner = importlib.import_module("annotationlib" if sys.version_info >= (3, 14) else "typing")
mode = sys.argv[1] if len(sys.argv) > 1 else ""
given = []
if mode == "patched":
    import builtins
    real = builtins.compile
    def patched(source, *args, **kwargs):
        if sys._getframe(1).f_globals is vars(owner):
            given.append(source)
        return real(source, *args, **kwargs)
    builtins.compile = patched
elif mode == "bound":
    owner.compile = compile
elif mode == "static":
    # A builtin with no `self` bound as `compile`.
    owner.compile = str.maketrans
used = collections.Counter()
def prof(frame, ev, arg):
    if ev == "c_call" and frame.f_globals is vars(owner) and getattr(arg, "__name__", "") == "compile":
        tables = getattr(arg, "__self__", None)
        text = owner._rewrite_star_unpack(frame.f_locals["arg"])
        used["ahead" if type(tables) is list and any(text in t for t in tables) else "text"] += 1
sys.setprofile(prof)
if mode == "static":
    try:
        import fm
    except Exception as e:
        print("import:", type(e).__name__, e)
    sys.setprofile(None)
    print("ahead-of-time:", used["ahead"], "from text:", used["text"], file=sys.stderr)
    sys.exit()
import fm, fe
sys.setprofile(None)
print("ahead-of-time:", used["ahead"], "from text:", used["text"], file=sys.stderr)
print(events)
print("compile bound:", "compile" in vars(owner), vars(owner).get("compile") is compile, given)
def ref(r):
    if isinstance(r, typing.ForwardRef):
        code = getattr(r, "__code__", None) if sys.version_info >= (3, 14) else r.__forward_code__
        return ("ref", r.__forward_arg__, r.__forward_is_argument__, r.__forward_is_class__,
                r.__forward_module__, code is not None and digest(code))
    args = typing.get_args(r)
    return (repr(typing.get_origin(r)), tuple(ref(a) for a in args)) if args else repr(r)
for mod in (fm, fe):
    for k, v in vars(mod).items():
        if isinstance(v, type) and v.__module__ == mod.__name__:
            print("==", k, v.__qualname__, v.__doc__ if k != "Pair" and k != "Point" else "-",
                  sorted(getattr(v, "__required_keys__", ())), sorted(getattr(v, "__optional_keys__", ())),
                  getattr(v, "_fields", None), getattr(v, "_field_defaults", None))
            for name, a in v.__annotations__.items():
                print("   ", name, ref(a))
print(ref(fe.explicit))
print(typing.get_type_hints(fe.Eager), typing.get_type_hints(fe.Pair))
print(typing.get_type_hints(fm.Point), fm.Point(1), fe.Pair(1, [fe.Pair(2)]))
print(fm.Child(req=1, extra=b"x"), fm.Config.__total__, fe.Eager.__total__)
code = lambda r: r.__forward_code__
print(code(fm.Twin1.__annotations__["a"]) is code(fm.Twin2.__annotations__["a"]),
      code(fm.Twin1.__annotations__["a"]) is code(fe.Again.__annotations__["a"]))
