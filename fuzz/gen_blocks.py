"""Feature blocks: self-contained snippets of one language feature with random holes.

Each block defines its own names (prefixed with the block's id, so blocks never
collide), uses them, and prints what it computed. The feature name of a block
is how refusals are counted per feature.
"""

from __future__ import annotations

from gen_core import (BOOL, FLOAT, FLOAT_EDGE, FLOAT_SPECS, INT, INT_EDGE, INT_SPECS, LINT, LSTR,
                      STR, STR_ASCII, STR_UNI, Core, Scope, fstring)
from gen_misc import MiscBlocks
from gen_stmts import IND, Stmts, indent


class Blocks(MiscBlocks):
    def __init__(self, c: Core) -> None:
        self.c = c
        self.r = c.r
        self.st = Stmts(c)
        self.imports: set[str] = set()

    def scope(self, p: str) -> Scope:
        s = Scope()
        s.prefix = p
        return s

    # ------------------------------------------------------------- compute
    def b_compute_top(self, p: str) -> list[str]:
        s = self.scope(p)
        return self.st.block(s, self.r.randint(3, 10), 2)

    def b_compute_func(self, p: str) -> list[str]:
        r = self.r
        s = self.scope(p)
        out: list[str] = []
        for i in range(r.randint(1, 2)):
            name = f"{p}f{i}"
            lines, ptys, rty = self.st.function(s, name)
            out += lines
            for _ in range(r.randint(1, 3)):
                args = ", ".join(self.c.expr(t, s, 1) for t in ptys)
                call = f"{name}({args})"
                out.append(f"print({call})" if rty != "None" else call)
        return out

    def b_recursion(self, p: str) -> list[str]:
        r, c = self.r, self.c
        s = self.scope(p)
        s.vars["n"] = INT
        op = r.choice(["+", "-", "*", "^"])
        base = c.int_lit()
        step = r.choice(["n - 1", "n // 2", "n - 2"])
        lines = [f"def {p}rec(n: int) -> int:",
                 f"{IND}if n <= 0:",
                 f"{IND * 2}return {base}",
                 f"{IND}return {p}rec({step}) {op} ({c.expr(INT, s, 1)})"]
        if r.random() < 0.5:
            lines += [f"def {p}fib(n: int) -> int:",
                      f"{IND}return n if n < 2 else {p}fib(n - 1) + {p}fib(n - 2)",
                      f"print([{p}fib(i) for i in range({r.randint(0, 20)})])"]
        lines.append(f"print({p}rec({r.randint(-1, 25)}))")
        return lines

    # -------------------------------------------------------------- numbers
    def b_int_table(self, p: str) -> list[str]:
        r = self.r
        vals = r.sample(INT_EDGE, 4) + [-v for v in r.sample(INT_EDGE, 3)]
        divs = [v for v in r.sample(INT_EDGE + [-3, -7, 5], 4) if v != 0] or [3]
        op = r.choice(["//", "%", "divmod", "**", "<<", ">>", "&", "|", "^", "str", "pow3", "cmp", "hash"])
        v, d = f"{p}a", f"{p}b"
        if op == "divmod":
            body = f"{p}q, {p}r = divmod({v}, {d})\nprint({v}, {d}, {p}q, {p}r, divmod({v}, {d}))"
        elif op == "**":
            body = f"print({v} ** {r.randint(0, 9)}, {d} ** 2)"
        elif op in ("<<", ">>"):
            body = f"print({v} {op} {r.randint(0, 130)}, {v} {op} (abs({d}) % 70))"
        elif op == "str":
            body = f"print(str({v}), repr({d}), int(str({v})) == {v}, len(str({v})), hex({v}), bin({d})[:20], oct({v}))"
        elif op == "pow3":
            body = f"print(pow({v}, {r.randint(0, 50)}, {d}))"
        elif op == "cmp":
            body = f"print({v} < {d}, {v} == {d}, {v} >= {d}, max({v}, {d}), min({v}, {d}), abs({v}), -{d}, {v} == float({d}))"
        elif op == "hash":
            body = f"print(hash({v}), hash({d}) == hash({d}), {v} * {d}, {v} - {d}, {v} + {d})"
        else:
            body = f"print({v} {op} {d}, -{v} {op} {d}, {v} {op} -{d})"
        # Each pair goes through a function, and the values through a list
        # of big `int`s.
        lines = [f"def {p}row({v}: int, {d}: int) -> None:"] + [IND + ln for ln in body.split("\n")]
        lines += [f"{p}row({x}, {y})" for x in vals for y in divs]
        lines += [f"{p}vals = [{', '.join(str(x) for x in vals)}]",
                  f"print({p}vals, sorted({p}vals), max({p}vals), {p}vals.index({vals[0]}))"]
        return lines

    def b_float_fmt(self, p: str) -> list[str]:
        r = self.r
        vals = r.sample(FLOAT_EDGE, 6)
        specs = r.sample(FLOAT_SPECS, 4)
        x = f"{p}x"
        lines = [f"{p}vals: list[float] = [{', '.join(vals)}]", f"for {x} in {p}vals:"]
        k = r.randrange(5)
        if k == 0:
            lines.append(f"{IND}print({x}, repr({x}), str({x}), {', '.join(f'format({x}, {sp!r})' for sp in specs)})")
        elif k == 1:
            fields = "|".join("{" + x + ":" + sp + "}" for sp in specs)
            lines.append(f"{IND}print({fstring(fields)})")
        elif k == 2:
            lines.append(f"{IND}print({x} + 1.0, {x} * -1.0, {x} / 3.0, {x} // 0.5, {x} % -2.5, -{x}, abs({x}), {x} == {x}, {x} != 0.0)")
        elif k == 3:
            lines.append(f"{IND}print(round({x}, {r.randint(-3, 5)}), {x} < 1.0, {x} > -1.0, max({x}, 0.0), min(0.0, {x}), {x} ** 2)")
        else:
            lines.append(f"{IND}if {x} == {x} and {x} - {x} == 0.0:")
            lines.append(f"{IND * 2}print(int({x}), round({x}), {x}.is_integer(), float(int({x})))")
            lines.append(f"{IND}else:")
            lines.append(f"{IND * 2}print('special', {x})")
        return lines

    def b_int_fmt(self, p: str) -> list[str]:
        r = self.r
        vals = r.sample(INT_EDGE, 5) + [-r.choice(INT_EDGE)]
        specs = [sp for sp in r.sample(INT_SPECS, 5) if sp != "c"]
        x = f"{p}x"
        fields = "|".join("{" + x + ":" + sp + "}" for sp in specs)
        return [f"def {p}show({x}: int) -> None:", f"{IND}print({fstring(fields)})",
                f"{IND}print({', '.join(f'format({x}, {sp!r})' for sp in specs)})"] + [f"{p}show({v})" for v in vals]

    # -------------------------------------------------------------- strings
    def b_str_ops(self, p: str) -> list[str]:
        r, c = self.r, self.c
        words = r.sample(STR_ASCII, 2) + r.sample(STR_UNI, 3)
        w = f"{p}w"
        s = self.scope(p)
        s.vars[w] = STR
        ops = []
        for _ in range(r.randint(2, 5)):
            k = r.randrange(12)
            if k == 0:
                ops.append(f"{w}[{r.randint(-6, 6)}:{r.randint(-6, 8)}:{r.choice([1, -1, 2, -2, 3])}]")
            elif k == 1:
                ops.append(f"({w}[{r.randint(-3, 3)}] if len({w}) > 3 else '')")
            elif k == 2:
                ops.append(f"{w}.find({c.str_lit()}), {w}.rfind({c.str_lit()}), {w}.count({c.str_lit()})")
            elif k == 3:
                ops.append(f"{w}.split({r.choice(['', repr(' '), repr(','), repr('a')])}), {w}.rsplit({r.choice(['', repr(' ')])})")
            elif k == 4:
                ops.append(f"{w}.center({r.randint(0, 14)}, {r.choice(['*', '-', 'é'])!r}), {w}.ljust({r.randint(0, 9)}), {w}.rjust({r.randint(0, 9)})")
            elif k == 5:
                ops.append(f"len({w}), {w}.replace({c.str_lit()}, {c.str_lit()}), {w} * {r.randint(0, 3)}")
            elif k == 6:
                ops.append(f"repr({w}), ascii({w})" if r.random() < 0.3 else f"repr({w})")
            elif k == 7:
                ops.append(f"{w}.startswith({c.str_lit()}), {w}.endswith({c.str_lit()})")
            elif k == 8:
                ops.append(f"[ch for ch in {w}][{r.randint(0, 2)}:]")
            elif k == 9:
                ops.append(f"{w} < {c.str_lit()}, {w} == {c.str_lit()}, {w} in {c.str_lit()}, {c.str_lit()} in {w}")
            elif k == 10:
                ops.append(f"{w}.strip(), {w}.lstrip({c.str_lit(True)}), {w}.rstrip()")
            else:
                ops.append(f"'-'.join({w}), {w}.upper(), {w}.lower()")
        return [f"for {w} in {words!r}:"] + indent([f"print({o})" for o in ops])

    def b_fstrings(self, p: str) -> list[str]:
        r, c = self.r, self.c
        s = self.scope(p)
        out = []
        for i, ty in enumerate([INT, FLOAT, STR, LINT]):
            n = f"{p}v{i}"
            out.append(f"{n}: {ty} = {c.expr(ty, s, 1)}")
            s.vars[n] = ty
        for _ in range(r.randint(2, 5)):
            fields = [c.fmt_field(s, 1) for _ in range(r.randint(1, 4))]
            out.append(f"print({fstring(r.choice([' ', ', ', '', '=']).join(fields))})")
        if r.random() < 0.4:
            out.append(f"print(f'{{{p}v0=}} {{{p}v1=:.2f}} {{{p}v2!r:>10}}')")
        return out

    # ----------------------------------------------------------- containers
    def b_containers(self, p: str) -> list[str]:
        r = self.r
        s = self.scope(p)
        out = []
        for ty in r.sample([LINT, LSTR, "dict[str, int]", "dict[int, str]", "set[int]", "list[float]", "tuple[int, str]"], 3):
            n = s.fresh()
            out.append(f"{n}: {ty} = {self.c.expr(ty, s, 2)}")
            s.vars[n] = ty
        out += self.st.block(s, r.randint(3, 8), 2)
        out.append(f"print({', '.join(n for n in s.vars)})")
        return out

    def b_comprehensions(self, p: str) -> list[str]:
        r, c = self.r, self.c
        s = self.scope(p)
        xs, ws = f"{p}xs", f"{p}ws"
        out = [f"{xs}: list[int] = {c.leaf(LINT, s)}", f"{ws}: list[str] = {c.expr(LSTR, s, 1)}"]
        k = r.randint(4, 7)
        opts = [
            f"[[a * b for b in range({r.randint(0, 4)})] for a in {xs}]",
            f"[a + b for a in {xs} for b in {xs} if a != b]",
            f"{{w: i for i, w in enumerate({ws})}}",
            f"{{a % {r.randint(2, 6)}: a for a in {xs}}}",
            f"sorted({{len(w) for w in {ws}}})",
            f"sum(a * a for a in {xs})",
            f"max((a for a in {xs}), default={c.int_lit(True)})",
            f"min([w for w in {ws}], default='none')",
            f"any(a > 3 for a in {xs}), all(a < 9 for a in {xs})",
            f"[(i, w) for i, w in enumerate({ws}) if i % 2 == 0]",
            f"list(zip({xs}, {ws}))",
            f"{{w: [ch for ch in w] for w in {ws}}}",
            f"[a if a > 0 else -a for a in {xs}]",
            f"sorted({xs}, key=lambda a: (a % 3, -a))",
            f"sorted({ws}, key=lambda w: (len(w), w), reverse=True)",
            f"[x for row in [[1, 2], [3], []] for x in row]",
            f"''.join(sorted(set(''.join({ws}))))",
            f"{{a for a in {xs} if a % 2 == 0}} | {{0}}",
            f"list(map(str, {xs}))",
            f"list(filter(lambda a: a % 2 == 1, {xs}))",
            f"[*{xs}, *range(3)]",
            f"{{**{{'a': 1}}, **{{w: 2 for w in {ws}}}}}",
        ]
        for e in r.sample(opts, k):
            out.append(f"print({e})")
        return out

    def b_collections(self, p: str) -> list[str]:
        r, c = self.r, self.c
        self.imports.add("from collections import Counter, defaultdict, deque")
        dq, ct, dd = f"{p}dq", f"{p}ct", f"{p}dd"
        out = [f"{dq}: deque[int] = deque({c.leaf(LINT, Scope())}{r.choice(['', ', maxlen=3', ', maxlen=5'])})",
               f"{ct}: Counter[str] = Counter({c.str_lit()})",
               f"{dd}: defaultdict[str, list[int]] = defaultdict(list)"]
        ops = [f"{dq}.append({c.int_lit(True)})", f"{dq}.appendleft({c.int_lit(True)})",
               f"if {dq}:\n{IND}print({dq}.popleft())", f"if {dq}:\n{IND}print({dq}.pop())",
               f"{dq}.rotate({r.randint(-3, 3)})", f"{dq}.extend([{c.int_lit(True)}, {c.int_lit(True)}])",
               f"print(list({dq}), len({dq}))", f"print({dq})",
               f"{ct}.update({c.str_lit()})", f"print({ct}.most_common({r.randint(0, 3)}))",
               f"print({ct}[{c.str_lit()}])", f"print(sorted({ct}.items()))", f"print({ct})",
               f"{dd}[{c.str_lit(True)}].append({c.int_lit(True)})", f"print(dict({dd}))",
               f"print(sorted({dd}.keys()), len({dd}))",
               f"{p}cw: defaultdict[str, int] = defaultdict(int)\nfor {p}ch in {c.str_lit()}:\n{IND}{p}cw[{p}ch] += 1\nprint(sorted({p}cw.items()))",
               f"print(sum({ct}.values()), {ct}.total())"]
        for o in r.choices(ops, k=r.randint(4, 9)):
            out += o.split("\n")
        return out

    def b_itertools(self, p: str) -> list[str]:
        r, c = self.r, self.c
        self.imports.add("import itertools")
        xs, ys = f"{p}xs", f"{p}ys"
        out = [f"{xs}: list[int] = {c.leaf(LINT, Scope())}", f"{ys}: list[str] = {c.leaf(LSTR, Scope())}"]
        opts = [
            f"list(itertools.chain({xs}, [7, 8]))",
            f"list(itertools.product({xs}, repeat=2))[:10]",
            f"list(itertools.permutations({xs}, 2))[:12]",
            f"list(itertools.combinations({xs}, 2))",
            f"list(itertools.combinations_with_replacement({xs}, 2))[:8]",
            f"list(itertools.accumulate({xs}))",
            f"list(itertools.islice(itertools.count({r.randint(-3, 3)}, {r.choice([1, 2, -1])}), {r.randint(0, 6)}))",
            f"list(itertools.zip_longest({xs}, {ys}))",
            f"list(itertools.islice({xs}, 1, None, 2))",
            f"[(k, len(list(g))) for k, g in itertools.groupby(sorted({xs}))]",
            f"list(itertools.repeat({c.int_lit(True)}, {r.randint(0, 4)}))",
            f"list(itertools.takewhile(lambda a: a < 3, {xs}))",
            f"list(itertools.dropwhile(lambda a: a < 3, {xs}))",
            f"list(itertools.pairwise({xs}))",
            f"list(itertools.product({ys}, {xs}))[:6]",
        ]
        for e in r.sample(opts, r.randint(3, 6)):
            out.append(f"print({e})")
        return out

    # -------------------------------------------------------------- classes
    def b_dataclass(self, p: str) -> list[str]:
        r, c = self.r, self.c
        self.imports.add("from dataclasses import dataclass, field")
        cls = f"{p.upper()}Pt"
        opts = r.choice(["", "(order=True)", "(frozen=True)", "(eq=False)"])
        fields = [("x", INT, c.int_lit(True)), ("y", INT, c.int_lit(True)), ("name", STR, c.str_lit()),
                  ("w", FLOAT, c.float_lit(True))]
        nf = r.randint(1, 4)
        fl = fields[:nf]
        lines = [f"@dataclass{opts}", f"class {cls}:"]
        ndef = r.randint(0, nf)
        for i, (n, t, d) in enumerate(fl):
            lines.append(f"{IND}{n}: {t}" + (f" = {d}" if i >= nf - ndef else ""))
        if opts != "(frozen=True)" and r.random() < 0.5:
            lines.append(f"{IND}tags: list[str] = field(default_factory=list)")
        lines += [f"{IND}def total(self) -> int:", f"{IND * 2}return {' + '.join('len(self.name)' if t == STR else ('int(self.w)' if t == FLOAT else 'self.' + n) for n, t, _ in fl)}"]
        mk = lambda: f"{cls}(" + ", ".join(c.int_lit(True) if t == INT else (c.str_lit() if t == STR else c.float_lit(True)) for _, t, _ in fl) + ")"  # noqa: E731
        a, b = f"{p}a", f"{p}b"
        lines += [f"{a} = {mk()}", f"{b} = {mk()}", f"print({a}, {b}, {a} == {b}, {a}.total())"]
        if opts == "(order=True)":
            lines.append(f"print({a} < {b}, sorted([{b}, {a}, {mk()}]))")
        if opts != "(frozen=True)" and r.random() < 0.6:
            lines += [f"{a}.{fl[0][0]} = {b}.{fl[0][0]}", f"print({a}, {a} == {b})"]
        if opts == "(frozen=True)":
            lines.append(f"print(len({{{a}, {b}, {a}}}), hash({a}) == hash({a}))")
        if r.random() < 0.3:
            lines.append(f"print(repr({a}), str({b}))")
        return lines

    def b_classes(self, p: str) -> list[str]:
        r, c = self.r, self.c
        B, D = f"{p.upper()}Base", f"{p.upper()}Derived"
        k = c.int_lit(True)
        lines = [f"class {B}:",
                 f"{IND}count: int = 0",
                 f"{IND}def __init__(self, v: int, tag: str) -> None:",
                 f"{IND * 2}self.v = v",
                 f"{IND * 2}self.tag = tag",
                 f"{IND * 2}{B}.count += 1",
                 f"{IND}def score(self) -> int:",
                 f"{IND * 2}return self.v * {k}",
                 f"{IND}def describe(self) -> str:",
                 f"{IND * 2}return f'{{self.tag}}:{{self.score()}}'"]
        dunders = r.sample(["repr", "str", "eq", "lt", "len", "add", "hash", "bool", "contains", "getitem"], r.randint(1, 5))
        if "hash" in dunders and "eq" not in dunders:
            dunders.append("eq")
        body = {
            "repr": [f"def __repr__(self) -> str:", f"{IND}return f'{B}({{self.v!r}}, {{self.tag!r}})'"],
            "str": [f"def __str__(self) -> str:", f"{IND}return '<' + self.tag + '>'"],
            "eq": ([f"def __eq__(self, other: object) -> bool:", f"{IND}return isinstance(other, {B}) and self.v == other.v"]
                   if r.random() < 0.4 else
                   [f"def __eq__(self, other: '{B}') -> bool:", f"{IND}return self.v == other.v"]),
            "lt": [f"def __lt__(self, other: '{B}') -> bool:", f"{IND}return self.v < other.v"],
            "len": [f"def __len__(self) -> int:", f"{IND}return len(self.tag)"],
            "add": [f"def __add__(self, other: '{B}') -> '{B}':", f"{IND}return {B}(self.v + other.v, self.tag + other.tag)"],
            "hash": [f"def __hash__(self) -> int:", f"{IND}return hash(self.v)"],
            "bool": [f"def __bool__(self) -> bool:", f"{IND}return self.v != 0"],
            "contains": [f"def __contains__(self, x: str) -> bool:", f"{IND}return x in self.tag"],
            "getitem": [f"def __getitem__(self, i: int) -> str:", f"{IND}return self.tag[i]"],
        }
        for d in dunders:
            lines += indent(body[d])
        lines += [f"class {D}({B}):",
                  f"{IND}def __init__(self, v: int, tag: str, extra: int) -> None:",
                  f"{IND * 2}super().__init__(v, tag)",
                  f"{IND * 2}self.extra = extra",
                  f"{IND}def score(self) -> int:",
                  f"{IND * 2}return super().score() + self.extra",
                  f"{IND}@property",
                  f"{IND}def doubled(self) -> int:",
                  f"{IND * 2}return self.extra * 2",
                  f"{IND}@staticmethod",
                  f"{IND}def helper(a: int) -> int:",
                  f"{IND * 2}return a - {c.int_lit(True)}",
                  f"{IND}@classmethod",
                  f"{IND}def make(cls, v: int) -> '{D}':",
                  f"{IND * 2}return cls(v, 'made', v + 1)"]
        a, b, d = f"{p}a", f"{p}b", f"{p}d"
        tag = r.choice(["ab", "xyz", "é日", ""])
        lines += [f"{a} = {B}({c.int_lit(True)}, {tag!r})",
                  f"{b} = {B}({c.int_lit(True)}, 'q')",
                  f"{d} = {D}({c.int_lit(True)}, 'dd', {c.int_lit(True)})",
                  f"print({a}.describe(), {b}.describe(), {d}.describe(), {d}.doubled, {D}.helper({c.int_lit(True)}))",
                  f"print({D}.make({c.int_lit(True)}).describe(), {B}.count, isinstance({d}, {B}), isinstance({a}, {D}))"]
        uses = {"repr": f"print(repr({a}), [{a}, {b}])", "str": f"print(str({a}), {a})", "eq": f"print({a} == {b}, {a} != {d}, {a} == {a})",
                "lt": f"print({a} < {b}, sorted([{a}, {b}, {d}], key=lambda o: o.v)[0].v)",
                "len": f"print(len({a}), len({d}))", "add": f"print(({a} + {b}).describe())",
                "hash": f"print(len({{{a}, {b}, {a}}}))", "bool": f"print(bool({a}), 'yes' if {b} else 'no')",
                "contains": f"print('a' in {a}, 'q' in {b})", "getitem": f"print({b}[0], {d}[-1])"}
        for dd in dunders:
            lines.append(uses[dd])
        return lines

    # ------------------------------------------------------------ functions
    def b_closure(self, p: str) -> list[str]:
        r, c = self.r, self.c
        k = r.randrange(3)
        if k == 0:
            return [f"def {p}counter(start: int) -> int:",
                    f"{IND}total = start",
                    f"{IND}def bump(by: int) -> None:",
                    f"{IND * 2}nonlocal total",
                    f"{IND * 2}total += by",
                    f"{IND}for i in range({r.randint(0, 5)}):",
                    f"{IND * 2}bump(i * {c.int_lit(True)})",
                    f"{IND}return total",
                    f"print({p}counter({c.int_lit()}))"]
        if k == 1:
            return [f"def {p}outer(n: int, s: str) -> str:",
                    f"{IND}def inner(k: int) -> str:",
                    f"{IND * 2}return s * (k % 3) + str(n + k)",
                    f"{IND}return '|'.join(inner(i) for i in range({r.randint(0, 4)}))",
                    f"print({p}outer({c.int_lit(True)}, {c.str_lit()}))"]
        return [f"def {p}make(n: int) -> list[int]:",
                f"{IND}acc: list[int] = []",
                f"{IND}def add(x: int) -> None:",
                f"{IND * 2}acc.append(x * n)",
                f"{IND}for v in {c.leaf(LINT, Scope())}:",
                f"{IND * 2}add(v)",
                f"{IND}return acc",
                f"print({p}make({c.int_lit(True)}))"]

    def b_generator(self, p: str) -> list[str]:
        r, c = self.r, self.c
        self.imports.add("from typing import Generator, Iterator")
        k = r.randrange(4)
        if k == 0:
            return [f"def {p}gen(n: int) -> Iterator[int]:",
                    f"{IND}for i in range(n):",
                    f"{IND * 2}if i % {r.randint(2, 4)} == 0:",
                    f"{IND * 3}continue",
                    f"{IND * 2}yield i * {c.int_lit(True)}",
                    f"{IND}yield -1",
                    f"print(list({p}gen({r.randint(0, 9)})), sum({p}gen(5)))",
                    f"{p}g = {p}gen({r.randint(0, 3)})",
                    f"print(next({p}g, 99), next({p}g, 99), next({p}g, 99))"]
        if k == 1:
            return [f"def {p}acc() -> Generator[int, int, str]:",
                    f"{IND}total = 0",
                    f"{IND}while True:",
                    f"{IND * 2}x = yield total",
                    f"{IND * 2}if x < 0:",
                    f"{IND * 3}return 'done ' + str(total)",
                    f"{IND * 2}total += x",
                    f"{p}g = {p}acc()",
                    f"print(next({p}g))",
                    f"for {p}v in {c.leaf(LINT, Scope())}:",
                    f"{IND}print({p}g.send({p}v % 10))",
                    f"try:",
                    f"{IND}{p}g.send(-1)",
                    f"except StopIteration as e:",
                    f"{IND}print('stop', e.value)"]
        if k == 2:
            return [f"def {p}inner(n: int) -> Iterator[str]:",
                    f"{IND}for i in range(n):",
                    f"{IND * 2}yield str(i)",
                    f"def {p}outer() -> Iterator[str]:",
                    f"{IND}yield 'start'",
                    f"{IND}yield from {p}inner({r.randint(0, 4)})",
                    f"{IND}yield 'end'",
                    f"print(list({p}outer()), '-'.join({p}outer()))"]
        return [f"def {p}gen() -> Generator[int, None, None]:",
                f"{IND}try:",
                f"{IND * 2}yield 1",
                f"{IND * 2}yield 2",
                f"{IND}except ValueError as e:",
                f"{IND * 2}print('gen caught', e)",
                f"{IND * 2}yield 10",
                f"{IND}finally:",
                f"{IND * 2}print('gen finally')",
                f"{p}g = {p}gen()",
                f"print(next({p}g))",
                f"print({p}g.throw(ValueError({c.str_lit()})))" if r.random() < 0.6 else f"print(next({p}g))",
                f"{p}g.close()",
                f"print('closed')"]

    def b_exceptions(self, p: str) -> list[str]:
        r, c = self.r, self.c
        E, F = f"{p.upper()}Err", f"{p.upper()}SubErr"
        lines = [f"class {E}(Exception):", f"{IND}pass", f"class {F}({E}):",
                 f"{IND}def __init__(self, code: int) -> None:",
                 f"{IND * 2}super().__init__(f'code {{code}}')",
                 f"{IND * 2}self.code = code",
                 f"def {p}risky(n: int) -> int:",
                 f"{IND}if n < 0:",
                 f"{IND * 2}raise {F}(n)",
                 f"{IND}if n == 0:",
                 f"{IND * 2}raise {E}({c.str_lit()})",
                 f"{IND}if n > 100:",
                 f"{IND * 2}raise ValueError('too big: ' + str(n))",
                 f"{IND}return n * 2"]
        for v in [c.int_lit(True), c.int_lit(True), r.choice(["0", "-5", "101", "7"])]:
            k = r.randrange(4)
            if k == 0:
                lines += ["try:", f"{IND}print({p}risky({v}))", f"except {F} as e:", f"{IND}print('sub', e.code, e)",
                          f"except {E} as e:", f"{IND}print('base', e, e.args)", "except ValueError as e:",
                          f"{IND}print('value', e)", "finally:", f"{IND}print('finally', {v})"]
            elif k == 1:
                lines += ["try:", f"{IND}try:", f"{IND * 2}{p}risky({v})", f"{IND}except {E} as e:",
                          f"{IND * 2}raise RuntimeError('wrapped') from e", "except RuntimeError as e:",
                          f"{IND}print(e, repr(e.__cause__))",
                          "except ValueError as e:", f"{IND}print('v', e)"]
            elif k == 2:
                lines += [f"def {p}loop{len(lines)}() -> int:", f"{IND}out = 0",
                          f"{IND}for i in range(3):", f"{IND * 2}try:", f"{IND * 3}out += {p}risky({v} + i)",
                          f"{IND * 3}if i == 1:", f"{IND * 4}return out", f"{IND * 2}except {E}:",
                          f"{IND * 3}out -= 1", f"{IND * 3}continue", f"{IND * 2}except ValueError:",
                          f"{IND * 3}break", f"{IND * 2}finally:", f"{IND * 3}print('fin', i)",
                          f"{IND}return out", f"print({p}loop{len(lines)}())"]
            else:
                lines += ["try:", f"{IND}{p}risky({v})", f"except ({F}, ValueError) as e:",
                          f"{IND}print('tuple', str(e))", f"except Exception as e:", f"{IND}print('any', repr(e))",
                          "else:", f"{IND}print('else')"]
        return lines

    def b_match(self, p: str) -> list[str]:
        r, c = self.r, self.c
        k = r.randrange(4)
        v = f"{p}v"
        if k == 0:
            vals = [c.int_lit(True) for _ in range(4)]
            return [f"for {v} in [{', '.join(vals)}]:",
                    f"{IND}match {v}:",
                    f"{IND * 2}case 0:", f"{IND * 3}print('zero')",
                    f"{IND * 2}case 1 | 2 | 3:", f"{IND * 3}print('small', {v})",
                    f"{IND * 2}case n if n < 0:", f"{IND * 3}print('neg', n)",
                    f"{IND * 2}case _:", f"{IND * 3}print('other', {v})"]
        if k == 1:
            return [f"for {v} in [{', '.join(c.str_lit(True) for _ in range(4))}]:",
                    f"{IND}match {v}:",
                    f"{IND * 2}case 'a' | 'abc':", f"{IND * 3}print('a-ish')",
                    f"{IND * 2}case '':", f"{IND * 3}print('empty')",
                    f"{IND * 2}case s if len(s) > 4:", f"{IND * 3}print('long', s)",
                    f"{IND * 2}case other:", f"{IND * 3}print('other', other)"]
        if k == 2:
            lists = [c.leaf(LINT, Scope()) for _ in range(4)]
            return [f"{p}rows: list[list[int]] = [{', '.join(lists)}]",
                    f"for {v} in {p}rows:",
                    f"{IND}match {v}:",
                    f"{IND * 2}case []:", f"{IND * 3}print('empty')",
                    f"{IND * 2}case [x]:", f"{IND * 3}print('one', x)",
                    f"{IND * 2}case [x, y]:", f"{IND * 3}print('two', x + y)",
                    f"{IND * 2}case [x, *rest]:", f"{IND * 3}print('many', x, rest)"]
        self.imports.add("from dataclasses import dataclass")
        P = f"{p.upper()}P"
        return [f"@dataclass", f"class {P}:", f"{IND}x: int", f"{IND}y: int",
                f"for {v} in [{P}({c.int_lit(True)}, {c.int_lit(True)}), {P}(0, 0), {P}(0, {c.int_lit(True)})]:",
                f"{IND}match {v}:",
                f"{IND * 2}case {P}(x=0, y=0):", f"{IND * 3}print('origin')",
                f"{IND * 2}case {P}(x=0, y=yy):", f"{IND * 3}print('on y', yy)",
                f"{IND * 2}case {P}(x=xx, y=yy) if xx == yy:", f"{IND * 3}print('diag', xx)",
                f"{IND * 2}case {P}(x=xx):", f"{IND * 3}print('x', xx)"]

    def b_varargs(self, p: str) -> list[str]:
        r, c = self.r, self.c
        k = r.randrange(3)
        if k == 0:
            return [f"def {p}total(*args: int) -> int:",
                    f"{IND}return sum(args) + len(args)",
                    f"print({p}total(), {p}total({c.int_lit(True)}), {p}total(*{c.leaf(LINT, Scope())}))"]
        if k == 1:
            return [f"def {p}kw(a: int, b: int = {c.int_lit(True)}, *, c: str = 'c') -> str:",
                    f"{IND}return f'{{a}}-{{b}}-{{c}}'",
                    f"print({p}kw(1), {p}kw(1, 2), {p}kw(1, c='z'), {p}kw(b=5, a=6), {p}kw({c.int_lit(True)}, c={c.str_lit()}))"]
        return [f"def {p}opts(name: str, **kwargs: int) -> str:",
                f"{IND}return name + ':' + ','.join(f'{{k}}={{v}}' for k, v in sorted(kwargs.items()))",
                f"print({p}opts('a'), {p}opts('b', x=1, y={c.int_lit(True)}), {p}opts(**{{'name': 'c', 'z': 3}}))"]

    def b_global(self, p: str) -> list[str]:
        c = self.c
        g = f"{p}G"
        return [f"{g}: int = {c.int_lit(True)}",
                f"{p}log: list[str] = []",
                f"def {p}bump(by: int) -> int:",
                f"{IND}global {g}",
                f"{IND}{g} += by",
                f"{IND}{p}log.append(str({g}))",
                f"{IND}return {g}",
                f"for {p}i in range({self.r.randint(0, 5)}):",
                f"{IND}{p}bump({p}i * {c.int_lit(True)})",
                f"print({g}, {p}log, {p}bump(0))"]


FEATURES = {
    "compute_top": 6, "compute_func": 6, "recursion": 2, "int_table": 3, "float_fmt": 3, "int_fmt": 2,
    "str_ops": 3, "fstrings": 3, "containers": 3, "comprehensions": 3, "collections": 2, "itertools": 2,
    "dataclass": 2, "classes": 3, "closure": 2, "generator": 2, "exceptions": 3, "match": 2,
    "varargs": 2, "global": 1, "idioms": 4, "match_more": 1,
}
