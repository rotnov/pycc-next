# pycc's profile driver (`pycc profile`, src/profile.rs), run by the target
# interpreter as `python -P -s driver.py MODE PKG OUT [CANDIDATES] -- SCRIPT [ARGS]`,
# with no `PYTHON*` variable in its environment but `PYTHONHASHSEED=0` (`-I`
# would ignore that too).
#
#   index  PKG OUT          every function of the package's sources: its module,
#                           qualified name, first line (its first decorator's, as
#                           `co_firstlineno`), the SHA-256 of its source text, its
#                           length in lines and whether it is eligible
#                           (docs/design/compile-little.md, section 2.1), as JSON
#                           in OUT.
#   calls  PKG OUT -- ...   runs SCRIPT under `cProfile`; writes, as JSON in OUT,
#                           the total self time of everything it saw and, per
#                           package function, its calls, self time and calls to
#                           Python functions by callee.
#   types  PKG OUT CANDIDATES -- ...
#                           runs SCRIPT with a `sys.monitoring` `PY_START` hook on
#                           the functions CANDIDATES (a JSON file of `[module,
#                           qualname, firstlineno]`) names; writes, per function,
#                           the exact types of its arguments in its first 1,000
#                           calls (type name to count, per parameter), and in
#                           how many of those it caught an exception in its own
#                           frame.
#   time   PKG OUT -- ...   runs SCRIPT once, nothing hooked, and writes its wall
#                           time in nanoseconds (pass 3, src/profile/verify.rs).
#                           PKG may be a built copy: its top-level name is
#                           found under it ahead of `sys.path` (so a SCRIPT
#                           that puts the sources on `sys.path` itself runs
#                           the copy, and nothing is imported before SCRIPT
#                           runs), and its modules are checked to come from
#                           under it, the package's own `__init__` as an
#                           extension too.
#
# SCRIPT runs as `__main__` (`runpy.run_path`) with `sys.argv` its own and the
# directory PKG is imported from (above every package holding it) first on
# `sys.path`, and that directory in `PYCC_PROFILE_ROOT` (pass 3's built copy's,
# for a workload that finds its test files by path); a `SystemExit` it raises ends it as it
# would end the interpreter. A pass whose workload imports no module of PKG,
# or one from elsewhere, fails.
import ast
import hashlib
import io
import json
import os
import re
import sys
import tokenize

TYPE_CALLS = 1000  # src/profile.rs's TYPE_CALLS


def import_root(pkg):
    """The directory PKG (a package directory, or a single module) is imported
    from: above every package that holds it, so a member `pkg/mod.py` is
    `pkg.mod`."""
    base = os.path.dirname(os.path.realpath(pkg))
    while os.path.isfile(os.path.join(base, "__init__.py")):
        base = os.path.dirname(base)
    return base


def dotted(base, path):
    parts = os.path.relpath(path, base)[:-3].split(os.sep)
    if parts[-1] == "__init__":
        parts.pop()
    return ".".join(parts)


def modules(pkg):
    """(dotted name, path) of every `.py` file under PKG (a package directory, or
    a single module), named as imported from `import_root(PKG)`."""
    base = import_root(pkg)
    pkg = os.path.realpath(pkg)
    if os.path.isfile(pkg):
        yield dotted(base, pkg), pkg
        return
    for root, dirs, files in os.walk(pkg):
        dirs.sort()
        for f in sorted(files):
            if f.endswith(".py"):
                path = os.path.join(root, f)
                yield dotted(base, path), path


def direct(node, kinds):
    """Whether `node`'s own scope (not a nested function, lambda or class) holds a
    node of `kinds`."""
    stack = list(ast.iter_child_nodes(node))
    while stack:
        n = stack.pop()
        if isinstance(n, kinds):
            return True
        if not isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda, ast.ClassDef)):
            stack.extend(ast.iter_child_nodes(n))
    return False


def functions(tree, lines):
    """(qualname, firstlineno, sha256, lines, eligible) of every function of a module, with
    CPython's qualified names."""
    out = []

    def walk(body, qual, in_function):
        for n in body:
            if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)):
                first = min([d.lineno for d in n.decorator_list] + [n.lineno])
                text = "".join(lines[first - 1:n.end_lineno])
                eligible = (not in_function and isinstance(n, ast.FunctionDef)
                            and not direct(n, (ast.Yield, ast.YieldFrom, ast.Await)))
                out.append((qual + n.name, first,
                            hashlib.sha256(text.encode("utf-8")).hexdigest(),
                            n.end_lineno - first + 1, eligible))
                walk(n.body, qual + n.name + ".<locals>.", True)
            elif isinstance(n, ast.ClassDef):
                walk(n.body, qual + n.name + ".", in_function)
            else:
                for field in ("body", "orelse", "finalbody", "handlers", "cases"):
                    sub = getattr(n, field, None)
                    if isinstance(sub, list):
                        walk([s for s in sub if isinstance(s, ast.AST)], qual, in_function)

    walk(tree.body, "", False)
    return out


def index(pkg):
    out = []
    for name, path in modules(pkg):
        with open(path, "rb") as f:
            raw = f.read()
        try:
            tree = ast.parse(raw, path)
        except SyntaxError:
            continue
        # In the module's own encoding (a `coding:` declaration, else UTF-8).
        encoding, _ = tokenize.detect_encoding(io.BytesIO(raw).readline)
        # Lines as `ast` counts them: ended only by `\r\n`, `\r` or `\n`
        # (`str.splitlines` also splits on `\f`, `\x85`, U+2028, ...).
        lines = re.split(r"(?<=\n)|(?<=\r)(?!\n)", raw.decode(encoding))
        for qual, first, digest, length, eligible in functions(tree, lines):
            out.append({"module": name, "qualname": qual, "firstlineno": first,
                        "sha256": digest, "lines": length, "eligible": eligible})
    return out


def run(pkg, script, args):
    import runpy
    sys.argv = [script, *args]
    sys.path.insert(0, os.path.dirname(os.path.abspath(script)))
    # PKG's sources, not a copy installed or beside the script. Its import root
    # is also the workload's to read (`PYCC_PROFILE_ROOT`): in pass 3 it is the
    # built copy's, where a suite that discovers its test files by path must
    # look (`unittest` refuses a test module imported from elsewhere).
    sys.path.insert(0, import_root(pkg))
    os.environ["PYCC_PROFILE_ROOT"] = import_root(pkg)
    # Pass 3's copy of a package: the modules beside the original (a
    # standalone module's siblings) are found where they are.
    origin = os.environ.pop("PYCC_PROFILE_ORIGIN", None)
    if origin is not None:
        sys.path.insert(1, import_root(origin))
    try:
        runpy.run_path(script, run_name="__main__")
    except SystemExit as e:
        if e.code not in (None, 0):
            raise


def check_loaded(pkg):
    """Fails unless the workload imported a module of PKG, every one from PKG. Run
    once the profiler is off, so its work is not in the profile."""
    loaded = 0
    for name, path in modules(pkg):
        m = sys.modules.get(name)
        if m is None:
            continue
        found = getattr(m, "__file__", None)
        if found is None or identity(found) != identity(path):
            sys.exit(f"pycc profile: the workload imported {name} from {found}, not from {path}")
        loaded += 1
    if not loaded:
        sys.exit(f"pycc profile: the workload imported no module of {pkg}")


def check_under(pkg):
    """Fails unless the workload imported a module of PKG, every one from a file
    under it: the built copy's package root is an extension, not its
    `__init__.py`."""
    real = os.path.realpath(pkg)
    loaded = 0
    for name, path in modules(pkg):
        m = sys.modules.get(name)
        if m is None:
            continue
        found = getattr(m, "__file__", None)
        if found is None:
            sys.exit(f"pycc profile: the workload imported {name} from no file, not from {pkg}")
        found = os.path.realpath(found)
        if os.path.isfile(real):
            stem = os.path.splitext(os.path.basename(real))[0]
            ok = (os.path.dirname(found) == os.path.dirname(real)
                  and os.path.basename(found).split(".")[0] == stem)
        else:
            ok = found.startswith(real + os.sep)
        if not ok:
            sys.exit(f"pycc profile: the workload imported {name} from {found}, not from {pkg}")
        loaded += 1
    if not loaded:
        sys.exit(f"pycc profile: the workload imported no module of {pkg}")


class CopyFinder:
    """Finds the top-level module `name` under `root` only (submodules follow
    its `__path__`)."""

    def __init__(self, name, root):
        self.name = name
        self.root = root

    def find_spec(self, fullname, path=None, target=None):
        if fullname != self.name:
            return None
        import importlib.machinery
        return importlib.machinery.PathFinder.find_spec(fullname, [self.root])


def timed(pkg, script, args):
    import time
    started = time.perf_counter_ns()
    # The copy's top-level name resolves under it first, so a workload that
    # puts the sources on `sys.path` itself still runs this copy; nothing is
    # imported before the workload, which may set up what its import reads.
    root = import_root(pkg)
    real = os.path.realpath(pkg)
    top = dotted(root, real if os.path.isfile(real)
                 else os.path.join(real, "__init__.py")).split(".")[0]
    sys.meta_path.insert(0, CopyFinder(top, root))
    run(pkg, script, args)
    ns = time.perf_counter_ns() - started
    check_under(pkg)
    return {"ns": ns}


_IDENTITY = {}


def identity(path):
    """The file `path` names, as `(st_dev, st_ino)`, or None: a code object's file
    name is the one the import system used, which on Windows can spell the same file
    another way (an 8.3 short name, another case, a `\\\\?\\` prefix)."""
    try:
        return _IDENTITY[path]
    except KeyError:
        pass
    try:
        st = os.stat(path)
        found = (st.st_dev, st.st_ino)
    except (OSError, ValueError, TypeError):
        found = None
    _IDENTITY[path] = found
    return found


def package_files(pkg):
    """The package's modules by file identity."""
    files = {identity(path): name for name, path in modules(pkg)}
    files.pop(None, None)
    return files


def calls(pkg, script, args):
    import cProfile
    files = package_files(pkg)
    prof = cProfile.Profile()
    prof.enable()
    try:
        run(pkg, script, args)
    finally:
        prof.disable()
    check_loaded(pkg)
    total = 0.0
    found = {}
    for e in prof.getstats():
        total += e.inlinetime
        code = e.code
        if isinstance(code, str):
            continue
        name = files.get(identity(code.co_filename))
        if name is None:
            continue
        key = f"{name}\0{code.co_qualname}\0{code.co_firstlineno}"
        c, t, out = found.get(key, (0, 0.0, {}))
        # Its calls to Python functions (a builtin's code is a string): a
        # function of the package by `module:qualname:firstlineno`, any
        # other as `*`.
        for sub in e.calls or ():
            if isinstance(sub.code, str):
                continue
            callee = files.get(identity(sub.code.co_filename))
            callee = "*" if callee is None else \
                f"{callee}:{sub.code.co_qualname}:{sub.code.co_firstlineno}"
            out[callee] = out.get(callee, 0) + sub.callcount
        found[key] = (c + e.callcount, t + e.inlinetime, out)
    return {"total_ns": round(total * 1e9),
            "functions": [[*k.split("\0"), c, round(t * 1e9), out]
                          for k, (c, t, out) in found.items()]}


def types(pkg, candidates, script, args):
    from _thread import get_ident
    files = package_files(pkg)
    wanted = {(m, q, int(f)) for m, q, f in candidates}
    # Per candidate: [calls sampled, argument types, sampled calls that caught
    # an exception, active calls by thread (each thread's innermost last, each
    # [sampled, counted]), sampling done, sampled calls still active].
    seen = {}
    by_code = {}
    mon = sys.monitoring
    tool = mon.PROFILER_ID

    def start(code, offset):
        rec = by_code.get(code)
        if rec is None:
            name = files.get(identity(code.co_filename))
            key = (name, code.co_qualname, code.co_firstlineno)
            if key not in wanted:
                return mon.DISABLE
            rec = seen.setdefault(key, [0, {}, 0, {}, False, 0])
            by_code[code] = rec
            mon.set_local_events(tool, code, mon.events.PY_RETURN)
        if rec[4]:
            return mon.DISABLE
        if rec[0] >= TYPE_CALLS:
            # No new call is sampled; the sampled ones still active are
            # followed until they end (an exception they handle counts), the
            # calls made meanwhile only kept in step, unsampled.
            if rec[5]:
                rec[3].setdefault(get_ident(), []).append([False, False])
                return None
            rec[4] = True
            rec[3].clear()
            mon.set_local_events(tool, code, 0)
            return mon.DISABLE
        rec[0] += 1
        rec[5] += 1
        rec[3].setdefault(get_ident(), []).append([True, False])
        frame = sys._getframe(1)
        n = code.co_argcount + code.co_kwonlyargcount
        n += bool(code.co_flags & 0x04) + bool(code.co_flags & 0x08)
        local = frame.f_locals
        for p in code.co_varnames[:n]:
            if p in local:
                t = type(local[p])
                tn = t.__qualname__ if t.__module__ == "builtins" else f"{t.__module__}.{t.__qualname__}"
                per = rec[1].setdefault(p, {})
                per[tn] = per.get(tn, 0) + 1
        return None

    def end(code, offset, value):
        rec = by_code.get(code)
        if rec is not None and not rec[4]:
            active = rec[3].get(get_ident())
            if active and active.pop()[0]:
                rec[5] -= 1
        return None

    def caught(code, offset, exc):
        # EXCEPTION_HANDLED (and PY_UNWIND) cannot be disabled before 3.15:
        # returning `DISABLE` from them raises `ValueError`.
        rec = by_code.get(code)
        if rec is None or rec[4]:
            return None
        active = rec[3].get(get_ident())
        if not active:
            return None
        # One count per sampled call: the innermost active one of this code
        # on this thread, where the handler runs.
        call = active[-1]
        if call[0] and not call[1]:
            call[1] = True
            rec[2] += 1
        return None

    mon.use_tool_id(tool, "pycc profile")
    mon.register_callback(tool, mon.events.PY_START, start)
    mon.register_callback(tool, mon.events.PY_RETURN, end)
    mon.register_callback(tool, mon.events.PY_UNWIND, end)
    mon.register_callback(tool, mon.events.EXCEPTION_HANDLED, caught)
    mon.set_events(tool, mon.events.PY_START | mon.events.PY_UNWIND | mon.events.EXCEPTION_HANDLED)
    try:
        run(pkg, script, args)
    finally:
        mon.set_events(tool, 0)
        for code in by_code:
            mon.set_local_events(tool, code, 0)
        mon.free_tool_id(tool)
    check_loaded(pkg)
    return {"functions": [[m, q, f, rec[1], rec[2]] for (m, q, f), rec in seen.items()]}


def main():
    argv = sys.argv[1:]
    sep = argv.index("--") if "--" in argv else len(argv)
    own, rest = argv[:sep], argv[sep + 1:]
    mode, pkg, out = own[0], own[1], own[2]
    if mode == "index":
        result = index(pkg)
    elif mode == "calls":
        result = calls(pkg, rest[0], rest[1:])
    elif mode == "time":
        result = timed(pkg, rest[0], rest[1:])
    else:
        with open(own[3], encoding="utf-8") as f:
            result = types(pkg, json.load(f), rest[0], rest[1:])
    with open(out, "w", encoding="utf-8") as f:
        json.dump(result, f)


main()
