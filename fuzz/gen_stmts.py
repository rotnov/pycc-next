"""Random typed statements (and the functions holding them) over `gen_core`."""

from __future__ import annotations

from gen_core import (BOOL, CONTAINERS, DIS, DSI, FLOAT, INT, LFLOAT, LINT, LSTR, MODS, SCALARS,
                      SINT, STR, TIS, Core, Scope)

IND = "    "


def indent(lines: list[str], n: int = 1) -> list[str]:
    return [IND * n + ln if ln else ln for ln in lines]


DECL_TYPES = [INT, INT, INT, FLOAT, FLOAT, BOOL, STR, STR, STR, LINT, LINT, LSTR, LFLOAT, DSI, DIS, SINT, TIS]

# Expressions that may raise, for `try` bodies and raising tails.
def risky(c: Core, s: Scope) -> str:
    r = c.r
    k = r.randrange(14)
    if k == 0:
        return f"print({c.expr(INT, s, 1)} // {c.expr(INT, s, 0)})"
    if k == 1:
        return f"print({c.expr(INT, s, 1)} % {c.r.choice(['0', c.expr(INT, s, 0)])})"
    if k == 2:
        return f"print({c.expr(FLOAT, s, 1)} / {c.r.choice(['0.0', '-0.0', c.expr(FLOAT, s, 0)])})"
    if k == 3:
        return f"print({c.expr(LINT, s, 1)}[{c.int_lit(True)}])"
    if k == 4:
        return f"print({c.expr(DSI, s, 1)}[{c.str_lit()}])"
    if k == 5:
        return f"print(int({r.choice(STR_BAD)!r}))"
    if k == 6:
        return f"print(float({r.choice(['x', '1.2.3', '', 'nan', '1e5', 'infinity', '-iNF'])!r}))"
    if k == 7:
        return f"print({c.expr(LINT, s, 1)}.index({c.int_lit(True)}))"
    if k == 8:
        return f"print({c.expr(STR, s, 1)}[{c.int_lit(True)}])"
    if k == 9:
        return f"print(int({c.expr(FLOAT, s, 1)}))"
    if k == 10:
        return f"print({c.expr(STR, s, 1)}.index({c.str_lit()}))"
    if k == 11:
        return f"print({r.choice(['min', 'max'])}({c.expr(LINT, s, 1)}))"
    if k == 12:
        return f"print({c.expr(DIS, s, 1)}[{c.int_lit(True)}])"
    return f"print(chr({c.expr(INT, s, 1)}))"


STR_BAD = ["abc", "", " ", "1.5", "0x10", "12a", "١٢٣", "--1", "1__0", "9" * 30, "-0", "+5"]

EXC_GROUPS = ["ZeroDivisionError", "IndexError", "KeyError", "ValueError", "OverflowError",
              "(ZeroDivisionError, ValueError)", "(IndexError, KeyError)", "ArithmeticError",
              "LookupError", "Exception"]


class Stmts:
    def __init__(self, c: Core) -> None:
        self.c = c
        self.r = c.r

    def block(self, s: Scope, n: int, depth: int) -> list[str]:
        out: list[str] = []
        for _ in range(n):
            out += self.stmt(s, depth)
        return out or ["pass"]

    def stmt(self, s: Scope, depth: int) -> list[str]:
        r = self.r
        weights = [("decl", 6), ("print", 6), ("assign", 3), ("aug", 3), ("mutate", 4),
                   ("if", 2 if depth > 0 else 0), ("for", 2 if depth > 0 else 0),
                   ("while", 1 if depth > 0 else 0), ("try", 1 if depth > 0 else 0),
                   ("swap", 1), ("brk", 1 if s.loop else 0)]
        kinds = [k for k, w in weights for _ in range(w)]
        return getattr(self, "s_" + r.choice(kinds))(s, depth)

    # ----------------------------------------------------------- statements
    def s_decl(self, s: Scope, depth: int) -> list[str]:
        ty = self.r.choice(DECL_TYPES)
        name = s.fresh()
        e = self.c.expr(ty, s, 2)
        s.vars[name] = ty
        return [f"{name}: {ty} = {e}"]

    def s_print(self, s: Scope, depth: int) -> list[str]:
        r = self.r
        args = [self.c.expr(r.choice(SCALARS + CONTAINERS), s, 2) for _ in range(r.randint(1, 3))]
        extra = r.choice(["", "", "", ", sep=''", ", sep=' | '", ", end=';\\n'", ", sep=','"])
        return [f"print({', '.join(args)}{extra})"]

    def bound(self, name: str, ty: str, e: str) -> str:
        """`e`, reduced so a rebinding in a loop (or from itself) cannot grow without bound."""
        if ty == INT:
            return f"({e}) % {self.r.choice(MODS)}"
        if ty in (STR, LINT, LSTR, LFLOAT):
            return f"({e})[:{self.r.randint(0, 30)}]"
        return e

    def assignable(self, s: Scope) -> list[str]:
        return [n for n in s.vars if n not in s.frozen]

    def s_assign(self, s: Scope, depth: int) -> list[str]:
        names = self.assignable(s)
        if not names:
            return self.s_decl(s, depth)
        name = self.r.choice(names)
        ty = s.vars[name]
        return [f"{name} = {self.bound(name, ty, self.c.expr(ty, s, 2))}"]

    def s_aug(self, s: Scope, depth: int) -> list[str]:
        r, c = self.r, self.c
        cands = [n for n in self.assignable(s) if s.vars[n] in (INT, FLOAT, STR, LINT, LSTR)]
        if not cands:
            return self.s_decl(s, depth)
        name = r.choice(cands)
        ty = s.vars[name]
        if ty == INT:
            op = r.choice(["+=", "-=", "*=", "//=", "%=", "&=", "|=", "^=", "<<=", ">>="])
            rhs = {"//=": c.nonzero_int(), "%=": c.nonzero_int(), "<<=": str(r.randint(0, 70)),
                   ">>=": str(r.randint(0, 70)), "*=": c.int_lit()}.get(op) or c.expr(INT, s, 1)
            return [f"{name} {op} {rhs}", f"{name} %= {r.choice(MODS)}"]
        if ty == FLOAT:
            op = r.choice(["+=", "-=", "*=", "/="])
            rhs = c.nonzero_float() if op == "/=" else c.expr(FLOAT, s, 1)
            return [f"{name} {op} {rhs}"]
        if ty == STR:
            return [f"{name} += {c.expr(STR, s, 1)}", f"{name} = {name}[:{r.randint(5, 40)}]"]
        return [f"{name} += {c.expr(ty, s, 1)}", f"{name} = {name}[:{r.randint(5, 30)}]"]

    def s_mutate(self, s: Scope, depth: int) -> list[str]:
        r, c = self.r, self.c
        cands = [n for n in self.assignable(s) if s.vars[n] in (LINT, LSTR, DSI, DIS, SINT, LFLOAT)]
        if not cands:
            return self.s_decl(s, depth)
        n = r.choice(cands)
        ty = s.vars[n]
        if ty in (LINT, LSTR, LFLOAT):
            el = {LINT: lambda: c.wrap_elem(s, 1), LSTR: lambda: c.expr(STR, s, 1),
                  LFLOAT: lambda: c.expr(FLOAT, s, 1)}[ty]
            k = r.randrange(10)
            if k == 0:
                return [f"{n}.append({el()})"]
            if k == 1:
                return [f"{n}.insert({c.int_lit(True)}, {el()})"]
            if k == 2:
                return [f"if {n}:", f"{IND}print({n}.pop({r.choice(['', '0', '-1'])}))"]
            if k == 3:
                return [f"{n}.extend({c.expr(ty, s, 1)})", f"del {n}[{r.randint(5, 30)}:]"]
            if k == 4:
                return [f"{n}.sort({r.choice(['', 'reverse=True'])})"]
            if k == 5:
                return [f"{n}.reverse()"]
            if k == 6:
                e = el()
                return [f"if {e} in {n}:", f"{IND}{n}.remove({e})"]
            if k == 7:
                return [f"if {n}:", f"{IND}del {n}[{c.expr(INT, s, 1)} % len({n})]"]
            if k == 8:
                return [f"if {n}:", f"{IND}{n}[{c.expr(INT, s, 1)} % len({n})] = {el()}"]
            return [f"{n}[{c.int_lit(True)}:{c.int_lit(True)}] = {c.expr(ty, s, 0)}", f"del {n}[30:]"]
        if ty in (DSI, DIS):
            key = (lambda: c.str_lit()) if ty == DSI else (lambda: c.int_lit(True))
            val = (lambda: c.wrap_elem(s, 1)) if ty == DSI else (lambda: c.expr(STR, s, 1))
            k = r.randrange(7)
            if k == 0:
                return [f"{n}[{key()}] = {val()}"]
            if k == 1:
                return [f"print({n}.pop({key()}, {val()}))"]
            if k == 2:
                kk = key()
                return [f"if {kk} in {n}:", f"{IND}del {n}[{kk}]"]
            if k == 3:
                return [f"print({n}.setdefault({key()}, {val()}))"]
            if k == 4:
                return [f"{n}.update({c.expr(ty, s, 1)})"]
            if k == 5:
                return [f"print({n}.get({key()}), {n}.get({key()}, {val()}))"]
            return [f"for kk in list({n}):", f"{IND}if len(str(kk)) % 2 == 0:", f"{IND * 2}del {n}[kk]"]
        k = r.randrange(5)
        if k == 0:
            return [f"{n}.add({c.wrap_elem(s, 1)})"]
        if k == 1:
            return [f"{n}.discard({c.int_lit(True)})"]
        if k == 2:
            e = c.int_lit(True)
            return [f"if {e} in {n}:", f"{IND}{n}.remove({e})"]
        if k == 3:
            return [f"{n}.update({c.expr(SINT, s, 1)})"]
        return [f"{n} {r.choice(['|=', '&=', '-=', '^='])} {c.expr(SINT, s, 1)}"]

    def s_if(self, s: Scope, depth: int) -> list[str]:
        r = self.r
        out = [f"if {self.c.expr(BOOL, s, 2)}:"] + indent(self.block(s.child(), r.randint(1, 3), depth - 1))
        for _ in range(r.randint(0, 2)):
            out += [f"elif {self.c.expr(BOOL, s, 1)}:"] + indent(self.block(s.child(), r.randint(1, 2), depth - 1))
        if r.random() < 0.5:
            out += ["else:"] + indent(self.block(s.child(), r.randint(1, 2), depth - 1))
        return out

    def loop_scope(self, s: Scope) -> Scope:
        inner = s.child()
        inner.loop += 1
        return inner

    def s_for(self, s: Scope, depth: int) -> list[str]:
        r, c = self.r, self.c
        inner = self.loop_scope(s)
        v = s.fresh("i")
        k = r.randrange(6)
        lists = [n for n in s.vars if s.vars[n] in (LINT, LSTR, LFLOAT)]
        dicts = [n for n in s.vars if s.vars[n] in (DSI, DIS)]
        if k == 1 and lists:
            src = r.choice(lists)
            inner.vars[v] = {LINT: INT, LSTR: STR, LFLOAT: FLOAT}[s.vars[src]]
            inner.frozen.add(src)
            head = f"for {v} in {src}:"
        elif k == 2 and dicts:
            src = r.choice(dicts)
            kt, vt = (STR, INT) if s.vars[src] == DSI else (INT, STR)
            v2 = s.fresh("j")
            inner.vars[v], inner.vars[v2] = kt, vt
            inner.frozen.add(src)
            head = f"for {v}, {v2} in {src}.items():"
        elif k == 3:
            v2 = s.fresh("j")
            inner.vars[v], inner.vars[v2] = INT, STR
            head = f"for {v}, {v2} in enumerate({c.expr(LSTR, s, 1)}{r.choice(['', ', 1', ', -2'])}):"
        elif k == 4:
            v2 = s.fresh("j")
            inner.vars[v], inner.vars[v2] = INT, STR
            head = f"for {v}, {v2} in zip({c.expr(LINT, s, 1)}, {c.expr(STR, s, 1)}):"
        elif k == 5:
            inner.vars[v] = STR
            head = f"for {v} in {c.expr(STR, s, 1)}:"
        else:
            inner.vars[v] = INT
            args = r.choice([f"{r.randint(0, 6)}", f"{r.randint(-3, 3)}, {r.randint(0, 7)}",
                             f"{r.randint(5, 9)}, {r.randint(-2, 2)}, -{r.randint(1, 3)}"])
            head = f"for {v} in range({args}):"
        inner.frozen.add(v)
        body = self.block(inner, r.randint(1, 3), depth - 1)
        out = [head] + indent(body)
        if r.random() < 0.2:
            out += ["else:"] + indent([f"print('for-else', {c.str_lit()})"])
        return out

    def s_while(self, s: Scope, depth: int) -> list[str]:
        r = self.r
        w = s.fresh("w")
        inner = self.loop_scope(s)
        inner.vars[w] = INT
        inner.frozen.add(w)
        cond = f" and {self.c.expr(BOOL, inner, 1)}" if r.random() < 0.4 else ""
        body = [f"{w} += 1"] + self.block(inner, r.randint(1, 3), depth - 1)
        return [f"{w} = 0", f"while {w} < {r.randint(0, 6)}{cond}:"] + indent(body)

    def s_brk(self, s: Scope, depth: int) -> list[str]:
        return [f"if {self.c.expr(BOOL, s, 1)}:", f"{IND}{self.r.choice(['break', 'continue'])}"]

    def s_try(self, s: Scope, depth: int) -> list[str]:
        r = self.r
        body = self.block(s.child(), r.randint(0, 2), depth - 1) + [risky(self.c, s)]
        out = ["try:"] + indent(body)
        for _ in range(r.randint(1, 2)):
            g = r.choice(EXC_GROUPS)
            pr = r.choice(["print('caught', type(e).__name__, e)", "print('caught', repr(e))",
                           "print('caught', str(e))", "print('caught', e.args)"])
            out += [f"except {g} as e:", f"{IND}{pr}"]
        if r.random() < 0.3:
            out += ["else:", f"{IND}print('no error')"]
        if r.random() < 0.3:
            out += ["finally:", f"{IND}print('finally')"]
        if not any(ln.startswith("except") for ln in out):
            out += ["except Exception:", f"{IND}pass"]
        return out

    def s_swap(self, s: Scope, depth: int) -> list[str]:
        r = self.r
        names = self.assignable(s)
        by_ty: dict[str, list[str]] = {}
        for n in names:
            by_ty.setdefault(s.vars[n], []).append(n)
        pairs = [v for v in by_ty.values() if len(v) >= 2]
        if pairs:
            a, b = r.sample(r.choice(pairs), 2)
            return [f"{a}, {b} = {b}, {a}"]
        q, m = s.fresh("q"), s.fresh("m")
        line = f"{q}, {m} = divmod({self.c.expr(INT, s, 1)}, {self.c.nonzero_int()})"
        s.vars[q], s.vars[m] = INT, INT
        return [line, f"print({q}, {m})"]

    # ------------------------------------------------------------ functions
    def function(self, s: Scope, name: str, depth: int = 2) -> tuple[list[str], list[str], str]:
        """A typed function; returns (definition lines, its parameter types, its return type)."""
        r = self.r
        fs = s.child()
        fs.vars = {}
        fs.frozen = set()
        fs.loop = 0
        params = []
        ptys = []
        for i in range(r.randint(0, 4)):
            ty = r.choice(DECL_TYPES)
            p = f"p{i}"
            fs.vars[p] = ty
            params.append(f"{p}: {ty}")
            ptys.append(ty)
        rty = r.choice(DECL_TYPES + ["None"])
        body = self.block(fs, r.randint(1, 6), depth)
        if rty != "None":
            body.append(f"return {self.c.expr(rty, fs, 2)}")
        return [f"def {name}({', '.join(params)}) -> {rty}:"] + indent(body), ptys, rty
