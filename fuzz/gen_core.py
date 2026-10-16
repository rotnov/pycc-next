"""Typed expression and statement generator: the core of the program generator.

Every expression is generated for a requested static type, from literals biased
toward boundary values, the variables in scope and operators whose result type
is that type, so a generated program type-checks in pycc's strict mode by
construction (a refusal then means an unsupported construct, not a type error).

Programs must terminate and stay small: loops have literal bounds, and an `int`,
`str` or list rebound from an expression that may grow (inside a loop, or from
itself) is reduced (`% m`, `[:k]`) so repeated rebinding cannot explode.
"""

from __future__ import annotations

import random

INT = "int"
FLOAT = "float"
BOOL = "bool"
STR = "str"
LINT = "list[int]"
LSTR = "list[str]"
LFLOAT = "list[float]"
DSI = "dict[str, int]"
DIS = "dict[int, str]"
SINT = "set[int]"
TIS = "tuple[int, str]"

SCALARS = [INT, FLOAT, BOOL, STR]
CONTAINERS = [LINT, LSTR, LFLOAT, DSI, DIS, SINT, TIS]
ALL_TYPES = SCALARS + CONTAINERS

# Boundary-biased literal pools.
INT_EDGE = [0, 1, -1, 2, -2, 3, 7, -7, 10, 255, 256, -128, 1000, 2**31 - 1, -(2**31), 2**31,
            2**32, 2**53, 2**53 + 1, 2**62, 2**63 - 1, -(2**63), 2**63, -(2**63) - 1, 2**64,
            2**64 - 1, 10**18, 10**19, 10**20, -(10**25), 3**40, 2**100 + 7]
INT_SMALL = list(range(-9, 10))
FLOAT_EDGE = ["0.0", "-0.0", "1.0", "-1.0", "0.1", "0.5", "-2.5", "1.5", "2.675", "1e16",
              "1e-7", "123456789.125", "1e22", "1e300", "-1e300", "5e-324", "2.2250738585072014e-308",
              "1.7976931348623157e+308", "0.30000000000000004", "1e15", "9007199254740993.0",
              "float('inf')", "float('-inf')", "float('nan')", "3.14159", "-0.001", "100.0"]
FLOAT_FINITE = [f for f in FLOAT_EDGE if "float(" not in f]
STR_ASCII = ["", "a", "ab", "abc", "Hello", "hello world", "MiXeD CaSe", "  padded  ", "a,b,,c",
             "x-y-z", "123", "-42", "3.5", "tab\there", "line\nbreak", "quote'd", 'dq"x', "back\\slash",
             "aaa", "The quick brown fox", "UPPER", "lower", "0", "Title Case Words", "a b  c"]
STR_UNI = ["héllo", "wörld", "日本語", "日本語テキスト", "😀", "a😀b", "😀😀x", "ñandú", "Ωmega",
           "naïve café", "€100", "привет", "δ", "é́", "中a文b", "\t  x"]
INT_SPECS = ["", "d", "5d", "<6", ">6", "^7", "08d", "+d", " d", "x", "X", "#x", "o", "#o", "b",
             "#b", ",", "_", "+,", "e", ".2f", "*^9", "-<5", "=+8", "c"]
FLOAT_SPECS = ["", ".0f", ".1f", ".2f", ".3f", "10.3f", "<10.2f", "e", ".3e", "E", "g", ".3g",
               ".12g", "G", "%", ".1%", "+.2f", ",", ",.2f", "_.1f", "08.2f", "^12", "n", ".0e",
               "#.0f", "-.3f", " .2f", "=+12.4f"]
STR_SPECS = ["", "<8", ">8", "^8", "*^9", ".2", "10.3", "-<6"]
MODS = ["97", "1000003", "-1000003", "2**61 - 1", "2**64 + 13", "12345678901234567890", "-7"]


def fstring(body: str) -> str:
    """An f-string literal around `body`, with a quote character it does not use."""
    for q in ('"', "'"):
        if q not in body:
            return "f" + q + body + q
    return 'f"""' + body + ' """'


class Scope:
    """Variables visible at a point, by type; `loop` is the loop nesting depth."""

    def __init__(self, parent: Scope | None = None) -> None:
        self.vars: dict[str, str] = dict(parent.vars) if parent else {}
        self.loop = parent.loop if parent else 0
        self.counter = parent.counter if parent else [0]
        self.prefix = parent.prefix if parent else "v"
        self.in_func = parent.in_func if parent else False
        self.frozen: set[str] = set(parent.frozen) if parent else set()

    def child(self) -> Scope:
        return Scope(self)

    def of(self, ty: str) -> list[str]:
        return [n for n, t in self.vars.items() if t == ty]

    def fresh(self, stem: str = "") -> str:
        self.counter[0] += 1
        return f"{self.prefix}{stem}{self.counter[0]}"


class Core:
    """Random typed expressions and statements over one `random.Random`."""

    def __init__(self, rng: random.Random) -> None:
        self.r = rng
        self.uses: set[str] = set()  # imports needed (`math`)

    # ---------------------------------------------------------------- literals
    def chance(self, p: float) -> bool:
        return self.r.random() < p

    def int_lit(self, small: bool = False) -> str:
        r = self.r
        if small or r.random() < 0.5:
            v = r.choice(INT_SMALL)
        elif r.random() < 0.7:
            v = r.choice(INT_EDGE)
            if r.random() < 0.3:
                v = -v
        else:
            v = r.randint(-(10**r.randint(1, 30)), 10**r.randint(1, 30))
        return f"({v})" if v < 0 else str(v)

    def float_lit(self, finite: bool = False) -> str:
        r = self.r
        if r.random() < 0.6:
            v = r.choice(FLOAT_FINITE if finite else FLOAT_EDGE)
        else:
            v = repr(round(r.uniform(-1000, 1000), r.randint(0, 6)))
        return f"({v})" if v.startswith("-") else v

    def str_lit(self, ascii_only: bool = False) -> str:
        pool = STR_ASCII if ascii_only or self.r.random() < 0.6 else STR_UNI
        return repr(self.r.choice(pool))

    def nonzero_int(self) -> str:
        return self.r.choice(["1", "2", "3", "7", "10", "(-1)", "(-3)", "(-7)", "(-10)", "1000000007",
                              "(-(2**64))", "2**63"])

    def nonzero_float(self) -> str:
        return self.r.choice(["2.0", "0.5", "(-1.5)", "3.0", "1e-3", "(-7.25)", "1e300", "0.1"])

    def small_int_expr(self, s: Scope, depth: int) -> str:
        """An `int` expression whose value fits easily in a machine word."""
        e = self.expr(INT, s, depth)
        return f"(({e}) % {self.r.choice(['10', '17', '100', '1000003'])})"

    # ------------------------------------------------------------ expressions
    def expr(self, ty: str, s: Scope, depth: int = 2) -> str:
        cands = s.of(ty)
        if depth <= 0 or self.r.random() < 0.2:
            if cands and self.r.random() < 0.6:
                return self.r.choice(cands)
            return self.leaf(ty, s)
        if cands and self.r.random() < 0.25:
            return self.r.choice(cands)
        return getattr(self, "e_" + {INT: "int", FLOAT: "float", BOOL: "bool", STR: "str", LINT: "lint",
                                     LSTR: "lstr", LFLOAT: "lfloat", DSI: "dsi", DIS: "dis", SINT: "sint",
                                     TIS: "tis"}[ty])(s, depth - 1)

    def leaf(self, ty: str, s: Scope) -> str:
        if ty == INT:
            return self.int_lit()
        if ty == FLOAT:
            return self.float_lit()
        if ty == BOOL:
            return self.r.choice(["True", "False"])
        if ty == STR:
            return self.str_lit()
        if ty == LINT:
            return "[" + ", ".join(self.int_lit(True) for _ in range(self.r.randint(0, 5))) + "]"
        if ty == LSTR:
            return "[" + ", ".join(self.str_lit() for _ in range(self.r.randint(0, 4))) + "]"
        if ty == LFLOAT:
            return "[" + ", ".join(self.float_lit(True) for _ in range(self.r.randint(0, 4))) + "]"
        if ty == DSI:
            keys = self.r.sample(STR_ASCII[:12] + STR_UNI[:4], self.r.randint(0, 4))
            return "{" + ", ".join(f"{k!r}: {self.int_lit(True)}" for k in keys) + "}"
        if ty == DIS:
            keys = self.r.sample(range(-5, 12), self.r.randint(0, 4))
            return "{" + ", ".join(f"{k}: {self.str_lit()}" for k in keys) + "}"
        if ty == SINT:
            n = self.r.randint(1, 5)
            return "{" + ", ".join(self.int_lit(True) for _ in range(n)) + "}"
        if ty == TIS:
            return f"({self.int_lit(True)}, {self.str_lit()})"
        raise AssertionError(ty)

    def e_int(self, s: Scope, d: int) -> str:
        r = self.r
        k = r.randrange(24)
        a = lambda: self.expr(INT, s, d)  # noqa: E731
        if k == 0:
            return f"({a()} {r.choice(['+', '-', '*'])} {a()})"
        if k == 1:
            return f"({a()} {r.choice(['//', '%'])} {self.nonzero_int()})"
        if k == 2:
            b = a()
            return f"({a()} {r.choice(['//', '%'])} ({b} or {self.nonzero_int()}))"
        if k == 3:
            base = self.int_lit() if r.random() < 0.6 else a()
            exp = r.randint(0, 40) if base[0] != "v" and len(base) < 8 else r.randint(0, 3)
            return f"({base} ** {exp})"
        if k == 4:
            return f"({a()} {r.choice(['<<', '>>'])} {r.randint(0, 100)})"
        if k == 5:
            return f"({a()} {r.choice(['&', '|', '^'])} {a()})"
        if k == 6:
            return f"{r.choice(['-', '~', '+'])}{a()}"
        if k == 7:
            return f"abs({a()})"
        if k == 8:
            return f"len({self.expr(r.choice([STR, LINT, LSTR, DSI, SINT]), s, d)})"
        if k == 9:
            return f"{r.choice(['min', 'max'])}({a()}, {a()})"
        if k == 10:
            return f"int({self.float_lit(True)})" if r.random() < 0.5 else f"int({self.expr(FLOAT, s, 0)})"
        if k == 11:
            return f"int({r.choice(['0', '-17', ' 42 ', '+7', '1_000', '99999999999999999999', '-0'])!r})"
        if k == 12:
            return f"{self.expr(STR, s, d)}.{r.choice(['find', 'count', 'rfind'])}({self.str_lit()})"
        if k == 13:
            return f"sum({self.expr(LINT, s, d)})"
        if k == 14:
            return f"({a()} if {self.expr(BOOL, s, d)} else {a()})"
        if k == 15:
            return f"round({self.expr(FLOAT, s, 0) if r.random() < 0.2 else self.float_lit(True)})"
        if k == 16:
            return f"{self.expr(LINT, s, d)}.count({self.int_lit(True)})"
        if k == 17:
            return f"int({self.expr(BOOL, s, d)})"
        if k == 18:
            st = self.expr(STR, s, d)
            return f"(ord({st}[0]) if {st} else -1)"
        if k == 19:
            return f"divmod({a()}, {self.nonzero_int()})[{r.randint(0, 1)}]"
        if k == 20:
            return f"pow({a()}, {r.randint(0, 20)}, {self.nonzero_int()})"
        if k == 21:
            self.uses.add("math")
            return f"math.{r.choice(['floor', 'ceil', 'trunc'])}({self.float_lit(True)})"
        if k == 22:
            return f"{self.expr(TIS, s, d)}[0]"
        return f"len({self.expr(STR, s, d)}) * {self.int_lit(True)}"

    def e_float(self, s: Scope, d: int) -> str:
        r = self.r
        k = r.randrange(16)
        a = lambda: self.expr(FLOAT, s, d)  # noqa: E731
        if k == 0:
            return f"({a()} {r.choice(['+', '-', '*'])} {a()})"
        if k == 1:
            return f"({a()} / {self.nonzero_float()})"
        if k == 2:
            return f"({a()} {r.choice(['//', '%'])} {self.nonzero_float()})"
        if k == 3:
            return f"float({self.expr(INT, s, d)})" if r.random() < 0.3 else f"float({self.int_lit(True)})"
        if k == 4:
            return f"({self.expr(INT, s, d)} / {self.nonzero_int()})"
        if k == 5:
            return f"{r.choice(['-', 'abs'])}({a()})"
        if k == 6:
            return f"{r.choice(['min', 'max'])}({a()}, {a()})"
        if k == 7:
            self.uses.add("math")
            return f"math.{r.choice(['fabs', 'sqrt', 'exp', 'floor'])}({r.choice(['2.0', '0.25', '10.0', '1e-5', '0.0', '-0.0'])}) * 1.0"
        if k == 8:
            return f"round({a()}, {r.randint(-2, 4)})"
        if k == 9:
            return f"({a()} ** {r.randint(0, 3)})"
        if k == 10:
            return f"({a()} if {self.expr(BOOL, s, d)} else {a()})"
        if k == 11:
            self.uses.add("math")
            return f"math.copysign({a()}, {self.float_lit()})"
        if k == 12:
            return f"({self.expr(INT, s, d)} * {a()})"
        if k == 13:
            return f"float({r.choice(['1.5', ' -2 ', 'inf', '-inf', 'nan', '1e500', '-0.0', '0x1p3'][:7])!r})"
        if k == 14:
            return f"({a()} - {self.float_lit()})"
        return f"(0.1 * {r.randint(1, 9)})"

    def e_bool(self, s: Scope, d: int) -> str:
        r = self.r
        k = r.randrange(14)
        if k == 0:
            t = r.choice([INT, FLOAT, STR])
            ops = ["<", "<=", ">", ">=", "==", "!="]
            return f"({self.expr(t, s, d)} {r.choice(ops)} {self.expr(t, s, d)})"
        if k == 1:
            return f"({self.expr(INT, s, d)} {r.choice(['<', '<=', '==', '!='])} {self.expr(FLOAT, s, d)})"
        if k == 2:
            return f"(not {self.expr(BOOL, s, d)})"
        if k == 3:
            return f"({self.expr(BOOL, s, d)} {r.choice(['and', 'or'])} {self.expr(BOOL, s, d)})"
        if k == 4:
            return f"({self.int_lit(True)} in {self.expr(r.choice([LINT, SINT]), s, d)})"
        if k == 5:
            return f"({self.str_lit()} in {self.expr(r.choice([STR, LSTR, DSI]), s, d)})"
        if k == 6:
            meth = r.choice(["startswith", "endswith"])
            return f"{self.expr(STR, s, d)}.{meth}({self.str_lit()})"
        if k == 7:
            meth = r.choice(["isdigit", "isalpha", "isspace", "isupper", "islower", "isalnum"])
            return f"{self.str_lit(ascii_only=True)}.{meth}()"
        if k == 8:
            return f"bool({self.expr(r.choice([INT, STR, LINT, FLOAT]), s, d)})"
        if k == 9:
            a = self.expr(INT, s, d)
            return f"({a} < {self.expr(INT, s, d)} <= {self.expr(INT, s, d)})"
        if k == 10:
            self.uses.add("math")
            return f"math.{r.choice(['isnan', 'isinf', 'isfinite'])}({self.expr(FLOAT, s, d)})"
        if k == 11:
            return f"{r.choice(['any', 'all'])}([x > {self.int_lit(True)} for x in {self.expr(LINT, s, d)}])"
        if k == 12:
            return f"({self.expr(LINT, s, d)} {r.choice(['==', '<', '!='])} {self.expr(LINT, s, d)})"
        return f"({self.expr(TIS, s, d)} {r.choice(['==', '<', '>='])} {self.expr(TIS, s, d)})"

    def fmt_field(self, s: Scope, d: int) -> str:
        r = self.r
        t = r.choice([INT, FLOAT, STR, BOOL, LINT])
        e = self.expr(t, s, min(d, 1))
        spec = ""
        if t == INT and r.random() < 0.7:
            spec = r.choice(INT_SPECS)
            if spec == "c":
                e = f"(65 + ({e}) % 26)"
        elif t == FLOAT and r.random() < 0.7:
            spec = r.choice(FLOAT_SPECS)
        elif t == STR and r.random() < 0.5:
            spec = r.choice(STR_SPECS)
        conv = r.choice(["", "", "", "!r"]) if t in (STR, FLOAT) else ""
        if conv and spec and t == FLOAT:
            conv = ""
        if conv == "!r" and t == STR:
            spec = r.choice(["", "<8", ">12"])
        return "{" + e + conv + (":" + spec if spec else "") + "}"

    def e_str(self, s: Scope, d: int) -> str:
        r = self.r
        k = r.randrange(22)
        a = lambda: self.expr(STR, s, d)  # noqa: E731
        if k == 0:
            return f"({a()} + {a()})"
        if k == 1:
            return f"({a()} * {r.randint(-1, 4)})"
        if k == 2:
            lo, hi = r.choice(["", self.int_lit(True)]), r.choice(["", self.int_lit(True)])
            step = r.choice(["", "", ":1", ":-1", ":2", ":-2", ":3"])
            return f"{a()}[{lo}:{hi}{step}]"
        if k == 3:
            st = a()
            return f"({st}[{self.expr(INT, s, d)} % len({st})] if {st} else '?')"
        if k == 4:
            return f"str({self.expr(r.choice([INT, FLOAT, BOOL, LINT, LSTR, TIS]), s, d)})"
        if k == 5:
            return f"repr({self.expr(r.choice([FLOAT, STR, INT]), s, d)})"
        if k == 6:
            meth = r.choice(["upper", "lower", "strip", "lstrip", "rstrip", "title", "capitalize",
                             "swapcase", "casefold"])
            return f"{self.str_lit(ascii_only=True) if r.random() < 0.6 else a()}.{meth}()"
        if k == 7:
            return f"{a()}.replace({self.str_lit()}, {self.str_lit()})"
        if k == 8:
            return f"{self.str_lit()}.join({self.expr(LSTR, s, d)})"
        if k == 9:
            fields = [self.fmt_field(s, d) for _ in range(r.randint(1, 3))]
            body = r.choice(["", " ", "-", "|", "é"]).join(fields)
            return fstring(body)
        if k == 10:
            meth = r.choice(["center", "ljust", "rjust"])
            fill = r.choice(["", ", '*'", ", '-'", ", 'é'"])
            return f"{a()}.{meth}({r.randint(-1, 12)}{fill})"
        if k == 11:
            return f"{a()}.zfill({r.randint(0, 10)})"
        if k == 12:
            return f"chr({r.choice(['65', '97', '0x4e00', '0x1f600', '233', '48'])} + ({self.expr(INT, s, d)}) % 26)"
        if k == 13:
            return f"({a()} if {self.expr(BOOL, s, d)} else {a()})"
        if k == 14:
            return f"{a()}.{r.choice(['removeprefix', 'removesuffix'])}({self.str_lit()})"
        if k == 15:
            spec = r.choice(INT_SPECS[:20])
            return f"format({self.expr(INT, s, d)}, {spec!r})"
        if k == 16:
            spec = r.choice(FLOAT_SPECS)
            return f"format({self.expr(FLOAT, s, d)}, {spec!r})"
        if k == 17:
            return f"'{{}} and {{}}'.format({self.expr(INT, s, d)}, {a()})"
        if k == 18:
            return f"{self.expr(TIS, s, d)}[1]"
        if k == 19:
            return f"{a()}.strip({self.str_lit(ascii_only=True)})"
        if k == 20:
            return f"str({self.expr(r.choice([DSI, SINT, LFLOAT]), s, d)})"
        return f"{a()}[::-1]"

    def wrap_elem(self, s: Scope, d: int) -> str:
        """An int element for a container: kept within a machine word."""
        if self.r.random() < 0.5:
            return self.int_lit(True)
        return self.small_int_expr(s, d)

    def e_lint(self, s: Scope, d: int) -> str:
        r = self.r
        k = r.randrange(14)
        a = lambda: self.expr(LINT, s, d)  # noqa: E731
        v = s.fresh("c")
        if k == 0:
            return "[" + ", ".join(self.wrap_elem(s, d) for _ in range(r.randint(0, 4))) + "]"
        if k == 1:
            inner = s.child()
            inner.vars[v] = INT
            cond = f" if {self.expr(BOOL, inner, 1)}" if r.random() < 0.4 else ""
            return f"[{self.small_int_expr(inner, 1)} for {v} in range({r.randint(-2, 8)}){cond}]"
        if k == 2:
            inner = s.child()
            inner.vars[v] = INT
            return f"[{v} * {self.int_lit(True)} for {v} in {a()} if {v} {r.choice(['>', '<', '!='])} {self.int_lit(True)}]"
        if k == 3:
            return f"list(range({r.randint(-5, 5)}, {r.randint(-5, 12)}, {r.choice(['1', '2', '3', '-1', '-2'])}))"
        if k == 4:
            return f"sorted({a()}{r.choice(['', ', reverse=True'])})"
        if k == 5:
            return f"({a()} + {a()})"
        if k == 6:
            return f"({a()} * {r.randint(-1, 3)})"
        if k == 7:
            lo, hi = r.choice(["", self.int_lit(True)]), r.choice(["", self.int_lit(True)])
            return f"{a()}[{lo}:{hi}{r.choice(['', ':-1', ':2'])}]"
        if k == 8:
            return f"sorted({self.expr(SINT, s, d)})"
        if k == 9:
            return f"list({self.expr(DIS, s, d)}.keys())" if r.random() < 0.5 else f"list({self.expr(DSI, s, d)}.values())"
        if k == 10:
            return f"[len(w) for w in {self.expr(LSTR, s, d)}]"
        if k == 11:
            return f"list(reversed({a()}))"
        if k == 12:
            return f"[x % 5 for x in {a()}]"
        return f"[ord(ch) % 50 for ch in {self.expr(STR, s, d)}]"

    def e_lstr(self, s: Scope, d: int) -> str:
        r = self.r
        k = r.randrange(9)
        if k == 0:
            return "[" + ", ".join(self.expr(STR, s, d) for _ in range(r.randint(0, 3))) + "]"
        if k == 1:
            sep = r.choice(["", "','", "' '", "'a'", "'é'"])
            return f"{self.expr(STR, s, d)}.split({sep})"
        if k == 2:
            return f"list({self.expr(STR, s, d)})"
        if k == 3:
            return f"sorted({self.expr(LSTR, s, d)}{r.choice(['', ', reverse=True', ', key=len'])})"
        if k == 4:
            return f"[w.{r.choice(['upper', 'strip', 'lower'])}() for w in {self.expr(LSTR, s, d)}]"
        if k == 5:
            return f"[str(x) for x in {self.expr(LINT, s, d)}]"
        if k == 6:
            return f"sorted({self.expr(DSI, s, d)})"
        if k == 7:
            return f"{self.expr(STR, s, d)}.splitlines()"
        return f"({self.expr(LSTR, s, d)} + {self.expr(LSTR, s, d)})"

    def e_lfloat(self, s: Scope, d: int) -> str:
        r = self.r
        k = r.randrange(4)
        if k == 0:
            return "[" + ", ".join(self.expr(FLOAT, s, d) for _ in range(r.randint(0, 3))) + "]"
        if k == 1:
            return f"[x / {self.nonzero_int()} for x in {self.expr(LINT, s, d)}]"
        if k == 2:
            return f"sorted({self.expr(LFLOAT, s, d)})"
        return f"[float(x) for x in {self.expr(LINT, s, d)}]"

    def e_dsi(self, s: Scope, d: int) -> str:
        r = self.r
        k = r.randrange(4)
        if k == 0:
            return self.leaf(DSI, s)
        if k == 1:
            return f"{{w: len(w) for w in {self.expr(LSTR, s, d)}}}"
        if k == 2:
            return f"dict(zip({self.expr(LSTR, s, d)}, {self.expr(LINT, s, d)}))"
        return f"{{str(x): x for x in {self.expr(LINT, s, d)}}}"

    def e_dis(self, s: Scope, d: int) -> str:
        if self.r.random() < 0.5:
            return self.leaf(DIS, s)
        return f"{{i: w for i, w in enumerate({self.expr(LSTR, s, d)})}}"

    def e_sint(self, s: Scope, d: int) -> str:
        r = self.r
        k = r.randrange(5)
        if k == 0:
            return self.leaf(SINT, s)
        if k == 1:
            return f"set({self.expr(LINT, s, d)})"
        if k == 2:
            return f"({self.expr(SINT, s, d)} {r.choice(['|', '&', '-', '^'])} {self.expr(SINT, s, d)})"
        if k == 3:
            return f"{{x % {r.randint(1, 9)} for x in {self.expr(LINT, s, d)}}}"
        return f"set({self.expr(DIS, s, d)})"

    def e_tis(self, s: Scope, d: int) -> str:
        return f"({self.wrap_elem(s, d)}, {self.expr(STR, s, d)})"
