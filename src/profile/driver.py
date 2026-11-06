# pycc's profile driver (`pycc profile`, src/profile.rs), run by the target
# interpreter as `python -I driver.py MODE PKG OUT [CANDIDATES] -- SCRIPT [ARGS]`.
#
#   index  PKG OUT          every function of the package's sources: its module,
#                           qualified name, first line (its first decorator's, as
#                           `co_firstlineno`), the SHA-256 of its source text and
#                           whether it is eligible (docs/design/compile-little.md,
#                           section 2.1), as JSON in OUT.
#   calls  PKG OUT -- ...   runs SCRIPT under `cProfile`; writes, as JSON in OUT,
#                           the total self time of everything it saw and, per
#                           package function, its calls, self time and calls to
#                           Python functions by callee.
#   types  PKG OUT CANDIDATES -- ...
#                           runs SCRIPT with a `sys.monitoring` `PY_START` hook on
#                           the functions CANDIDATES (a JSON file of `[module,
#                           qualname, firstlineno]`) names; writes, per function,
#                           the exact types of its arguments in its first 1,000
#                           calls (type name to count, per parameter).
#
# SCRIPT runs as `__main__` (`runpy.run_path`) with `sys.argv` its own; a
# `SystemExit` it raises ends it as it would end the interpreter.
import ast
import hashlib
import json
import os
import sys

TYPE_CALLS = 1000


def modules(pkg):
    """(dotted name, path) of every `.py` file under PKG (a package directory, or
    a single module)."""
    pkg = os.path.realpath(pkg)
    if os.path.isfile(pkg):
        yield os.path.splitext(os.path.basename(pkg))[0], pkg
        return
    base = os.path.dirname(pkg)
    for root, dirs, files in os.walk(pkg):
        dirs.sort()
        for f in sorted(files):
            if f.endswith(".py"):
                path = os.path.join(root, f)
                parts = os.path.relpath(path, base)[:-3].split(os.sep)
                if parts[-1] == "__init__":
                    parts.pop()
                yield ".".join(parts), path


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
    """(qualname, firstlineno, sha256, eligible) of every function of a module, with
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
                            hashlib.sha256(text.encode("utf-8")).hexdigest(), eligible))
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
        lines = raw.decode("utf-8").splitlines(keepends=True)
        for qual, first, digest, eligible in functions(tree, lines):
            out.append({"module": name, "qualname": qual, "firstlineno": first,
                        "sha256": digest, "eligible": eligible})
    return out


def run(script, args):
    import runpy
    sys.argv = [script, *args]
    sys.path.insert(0, os.path.dirname(os.path.abspath(script)))
    try:
        runpy.run_path(script, run_name="__main__")
    except SystemExit as e:
        if e.code not in (None, 0):
            raise


def package_files(pkg):
    return {path: name for name, path in modules(pkg)}


def calls(pkg, script, args):
    import cProfile
    files = package_files(pkg)
    prof = cProfile.Profile()
    prof.enable()
    try:
        run(script, args)
    finally:
        prof.disable()
    total = 0.0
    found = {}
    for e in prof.getstats():
        total += e.inlinetime
        code = e.code
        if isinstance(code, str):
            continue
        name = files.get(os.path.realpath(code.co_filename))
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
            callee = files.get(os.path.realpath(sub.code.co_filename))
            callee = "*" if callee is None else \
                f"{callee}:{sub.code.co_qualname}:{sub.code.co_firstlineno}"
            out[callee] = out.get(callee, 0) + sub.callcount
        found[key] = (c + e.callcount, t + e.inlinetime, out)
    return {"total_ns": round(total * 1e9),
            "functions": [[*k.split("\0"), c, round(t * 1e9), out]
                          for k, (c, t, out) in found.items()]}


def types(pkg, candidates, script, args):
    files = package_files(pkg)
    wanted = {(m, q, int(f)) for m, q, f in candidates}
    seen = {}
    mon = sys.monitoring
    tool = mon.PROFILER_ID

    def start(code, offset):
        name = files.get(os.path.realpath(code.co_filename))
        key = (name, code.co_qualname, code.co_firstlineno)
        if key not in wanted:
            return mon.DISABLE
        rec = seen.setdefault(key, [0, {}])
        if rec[0] >= TYPE_CALLS:
            return mon.DISABLE
        rec[0] += 1
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

    mon.use_tool_id(tool, "pycc profile")
    mon.register_callback(tool, mon.events.PY_START, start)
    mon.set_events(tool, mon.events.PY_START)
    try:
        run(script, args)
    finally:
        mon.set_events(tool, 0)
        mon.free_tool_id(tool)
    return {"functions": [[m, q, f, rec[1]] for (m, q, f), rec in seen.items()]}


def main():
    argv = sys.argv[1:]
    sep = argv.index("--") if "--" in argv else len(argv)
    own, rest = argv[:sep], argv[sep + 1:]
    mode, pkg, out = own[0], own[1], own[2]
    if mode == "index":
        result = index(pkg)
    elif mode == "calls":
        result = calls(pkg, rest[0], rest[1:])
    else:
        with open(own[3], encoding="utf-8") as f:
            result = types(pkg, json.load(f), rest[0], rest[1:])
    with open(out, "w", encoding="utf-8") as f:
        json.dump(result, f)


main()
