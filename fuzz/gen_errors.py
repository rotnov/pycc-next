"""Error-text blocks (v0.10 accept 1, issue #10): calls of compiled functions,
methods and constructors with missing, extra, unknown, misspelled or repeated
arguments, and reads of missing attributes of classes and instances.

Each wrong call or read runs in a `try` and prints the exception's type and
message, so the comparison is of CPython's exact text (an uncaught `TypeError`
or `AttributeError` would make the program a discarded generator bug). A
missing attribute's "Did you mean" is printed through
`traceback.format_exception_only`, which reads the exception's `name` and
`obj` as an uncaught one's traceback does.

Strict modes refuse each wrong call at compile time (`T0021`), which prunes it;
non-strict modes run it in CPython against the published compiled function,
which is the path whose messages this compares. Shapes `docs/DEVIATIONS.md`
lists are not generated: a method called through its class with no receiver
or another type's (`C.m()`, `C.m(5)`), and an instance field read through the
class (`C.field`).
"""

from __future__ import annotations

from gen_core import Core
from gen_stmts import IND

# Parameter types and a literal of each.
TYPES = {"int": lambda c: c.int_lit(True), "str": lambda c: repr(c.r.choice(["a", "bc", ""])),
         "float": lambda c: repr(c.r.choice([0.5, -2.0, 3.25]))}
NAMES = ["x", "y", "count", "label", "alpha", "beta", "width", "height", "value", "key", "n", "item"]


def misspell(r, name: str) -> str:
    """A near miss of `name` (CPython may suggest `name`) or an unrelated name."""
    k = r.randrange(5)
    if k == 0 and len(name) > 1:
        i = r.randrange(len(name))
        return name[:i] + name[i + 1:]
    if k == 1 and len(name) > 1:
        i = r.randrange(len(name) - 1)
        return name[:i] + name[i + 1] + name[i] + name[i + 2:]
    if k == 2:
        return name + r.choice(["s", "_", "2"])
    if k == 3:
        return name.upper() if name.upper() != name else name + "X"
    return r.choice(["zz", "other", "missing", "qq"])


def suggests(candidates: list[str], name: str) -> str | None:
    """CPython's "Did you mean" pick for `name` among `candidates`."""
    try:
        import _suggestions
    except ImportError:  # not CPython 3.13: be conservative
        return candidates[0]
    return _suggestions._generate_suggestions(candidates, name)


class Sig:
    """A generated signature: positional parameters (the last `defaults` of
    them defaulted) and keyword-only ones (each defaulted or not)."""

    def __init__(self, c: Core, receiver: str | None, allow_defaults: bool, allow_kwonly: bool) -> None:
        r = c.r
        self.c = c
        self.receiver = receiver
        n = r.randint(0, 3)
        names = r.sample(NAMES, n + 2)
        self.params = [(names[i], r.choice(list(TYPES))) for i in range(n)]
        self.defaults = r.randint(0, n) if allow_defaults and r.random() < 0.4 else 0
        self.default_values = [TYPES[t](c) for _, t in self.params[n - self.defaults:]]
        self.kwonly: list[tuple[str, str, str | None]] = []
        if allow_kwonly and r.random() < 0.25:
            for name in names[n:n + r.randint(1, 2)]:
                t = r.choice(list(TYPES))
                self.kwonly.append((name, t, TYPES[t](c) if r.random() < 0.5 else None))

    def header(self, name: str) -> str:
        parts = [self.receiver] if self.receiver else []
        first_default = len(self.params) - self.defaults
        for i, (p, t) in enumerate(self.params):
            parts.append(f"{p}: {t}" + (f" = {self.default_values[i - first_default]}" if i >= first_default else ""))
        if self.kwonly:
            parts.append("*")
            parts += [f"{p}: {t}" + (f" = {d}" if d is not None else "") for p, t, d in self.kwonly]
        return f"def {name}({', '.join(parts)}) -> str:"

    def body(self) -> str:
        names = [p for p, _ in self.params] + [p for p, _, _ in self.kwonly]
        return f"{IND}return {' + '.join(f'str({n})' for n in names) or repr('-')}"

    def arg(self, t: str) -> str:
        return TYPES[t](self.c)

    def calls(self) -> list[str]:
        """Argument lists: a valid one and several that cannot bind."""
        r = self.c.r
        n, req = len(self.params), len(self.params) - self.defaults
        req_kw = [(p, t) for p, t, d in self.kwonly if d is None]
        kwonly = ", ".join(f"{p}={self.arg(t)}" for p, t in req_kw)

        def join(*parts: str) -> str:
            return ", ".join(x for x in parts if x)

        pos = lambda k: ", ".join(self.arg(t) for _, t in self.params[:k])  # noqa: E731
        out = [join(pos(r.randint(req, n)), kwonly)]
        # Too few positional arguments.
        if req > 0:
            out.append(join(pos(r.randint(0, req - 1)), kwonly))
        # Too many.
        extra = ", ".join(self.arg(r.choice(list(TYPES))) for _ in range(r.randint(1, 2)))
        out.append(join(pos(n), extra, kwonly if r.random() < 0.5 else ""))
        # By keyword, in a shuffled order, one left out sometimes.
        if n:
            named = [f"{p}={self.arg(t)}" for p, t in self.params]
            r.shuffle(named)
            if r.random() < 0.4:
                named.pop()
            out.append(join(", ".join(named), kwonly))
        # An unknown or misspelled keyword.
        target = r.choice([p for p, _ in self.params] + [p for p, _, _ in self.kwonly]
                          + ([self.receiver] if self.receiver else []) + ["zz"])
        out.append(join(pos(req), f"{misspell(r, target)}={self.arg('int')}", kwonly))
        # A parameter given positionally and by keyword.
        if n:
            k = r.randint(1, n)
            p, t = self.params[r.randrange(k)]
            out.append(join(pos(k), f"{p}={self.arg(t)}", kwonly))
        # A keyword-only parameter left out.
        if req_kw:
            out.append(pos(req))
        r.shuffle(out)
        return out[:r.randint(3, len(out))]


class ErrorBlocks:
    c: Core
    imports: set[str]

    def b_call_errors(self, p: str) -> list[str]:
        c, r = self.c, self.c.r
        lines: list[str] = []
        probes: list[str] = []
        f = f"{p}fn"
        fs = Sig(c, None, allow_defaults=True, allow_kwonly=True)
        lines += [fs.header(f), fs.body()]
        probes += [f"{f}({a})" for a in fs.calls()]
        C = f"{p.upper()}K"
        field = r.choice(["w", "size", "total", "name"])
        init = Sig(c, "self", allow_defaults=r.random() < 0.5, allow_kwonly=False)
        meth = Sig(c, "self", allow_defaults=r.random() < 0.5, allow_kwonly=False)
        stat = Sig(c, None, allow_defaults=False, allow_kwonly=False)
        clsm = Sig(c, "cls", allow_defaults=False, allow_kwonly=False)
        m, s, k = r.choice(["area", "scale", "get", "render"]), r.choice(["unit", "build"]), "make"
        lines += [f"class {C}:", IND + init.header("__init__").replace("-> str:", "-> None:"),
                  f"{IND * 2}self.{field} = {len(init.params)}",
                  IND + meth.header(m), IND + meth.body(),
                  f"{IND}@staticmethod", IND + stat.header(s), IND + stat.body(),
                  f"{IND}@classmethod", IND + clsm.header(k), IND + clsm.body()]
        good = "".join(f"{init.arg(t)}, " for _, t in init.params[:len(init.params) - init.defaults])
        o = f"{p}obj"
        lines.append(f"{o} = {C}({good.rstrip(', ')})")
        probes += [f"{C}({a}).{field}" for a in init.calls()]
        probes += [f"{o}.{m}({a})" for a in meth.calls()]
        probes += [f"{C}.{m}({o}{', ' + a if a else ''})" for a in meth.calls()[:2]]
        probes += [f"{r.choice([C, o])}.{s}({a})" for a in stat.calls()]
        probes += [f"{r.choice([C, o])}.{k}({a})" for a in clsm.calls()]
        r.shuffle(probes)
        probes = probes[:r.randint(4, 12)]
        for call in probes:
            lines += ["try:", f"{IND}print({call})", "except TypeError as e:", f"{IND}print('TypeError', e)"]
        # Missing attributes: near misses of the members, and unrelated names.
        self.imports.add("import traceback")
        members = [field, m, s, k]
        for _ in range(r.randint(1, 4)):
            owner = r.choice([C, o])
            # The class's own reads never misspell the instance field: the
            # field's descriptor is on a compiled class (DEVIATIONS), so
            # CPython's "Did you mean" would differ.
            name = misspell(r, r.choice(members if owner == o else members[1:]))
            if name in members or (owner == C and suggests(members, name) == field):
                continue
            read = r.choice([f"{owner}.{name}", f"getattr({owner}, {name!r})"])
            lines += ["try:", f"{IND}print({read})", "except AttributeError as e:",
                      f"{IND}print(traceback.format_exception_only(e)[-1].rstrip())"]
            if r.random() < 0.3:
                lines.append(f"print(hasattr({owner}, {name!r}), getattr({owner}, {name!r}, 'none'))")
        return lines
