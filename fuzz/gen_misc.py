"""More feature blocks: assorted idioms with CPython-specific edges.

Each snippet is one small, self-contained shape (its names prefixed by the
block's id) with random values in its holes; a block picks a few of them.
"""

from __future__ import annotations

from gen_core import INT_EDGE, LINT, STR_ASCII, STR_UNI, Core, Scope
from gen_stmts import IND


class MiscBlocks:
    c: Core

    def b_idioms(self, p: str) -> list[str]:
        r, c = self.c.r, self.c
        s = Scope()
        snippets = [
            # try/finally whose finally overrides the return value.
            lambda: [f"def {p}fin(n: int) -> int:", f"{IND}try:", f"{IND * 2}if n > 0:",
                     f"{IND * 3}return n", f"{IND * 2}return -n", f"{IND}finally:",
                     f"{IND * 2}print('finally', n)", f"print({p}fin({c.int_lit(True)}), {p}fin(0))"],
            # Slice assignment, extended slices, deletion by slice.
            lambda: [f"{p}xs: list[int] = list(range({r.randint(0, 12)}))",
                     f"{p}xs[{r.randint(-3, 3)}:{r.randint(-3, 8)}] = [{c.int_lit(True)}, {c.int_lit(True)}]",
                     f"del {p}xs[::{r.choice([2, 3, -2])}]",
                     f"print({p}xs, {p}xs[::-1], {p}xs[{r.randint(-5, 5)}:{r.randint(-5, 9)}:{r.choice([1, 2, -1, -3])}])"],
            # Dict order after deletion and re-insertion; popitem; `|`.
            lambda: [f"{p}d: dict[str, int] = {{'a': 1, 'b': 2, 'c': 3}}",
                     f"del {p}d['a']", f"{p}d['a'] = {c.int_lit(True)}",
                     f"print({p}d, list({p}d), {p}d.popitem(), {p}d | {{'z': 0, 'b': 9}})"],
            # Integer parsing and printing in other bases.
            lambda: [f"print(int({r.choice(['ff', '-7f', '0x1f', '777', '1010', 'zz', '_1'])!r}, "
                     f"{r.choice([16, 0, 8, 2, 36])}) if {r.choice(['True', 'False'])} else 0, "
                     f"hex({c.int_lit()}), oct({c.int_lit()}), bin({c.int_lit()}))"],
            # Float division and modulo by negatives, divmod of floats.
            lambda: [f"for {p}a in [{c.float_lit(True)}, {c.float_lit(True)}, -0.0, 7.5]:",
                     f"{IND}for {p}b in [-2.0, 0.5, -0.25, 3.0]:",
                     f"{IND * 2}print({p}a // {p}b, {p}a % {p}b, divmod({p}a, {p}b), {p}a / {p}b)"],
            # Booleans as numbers, comparisons of int and float.
            lambda: [f"print(True + True, True * {c.int_lit(True)}, -True, ~False, True / 2, 2 ** 64 == 2.0 ** 64, "
                     f"{c.int_lit()} == {c.float_lit(True)}, 2 ** 53 + 1 == float(2 ** 53 + 1), 0.1 + 0.2 == 0.3)"],
            # str methods with arguments.
            lambda: [f"{p}t: str = {c.str_lit()}",
                     f"print({p}t.split({r.choice([repr(' '), repr(','), repr('a')])}, {r.randint(-1, 2)}), "
                     f"{p}t.rsplit(None, {r.randint(0, 2)}), {p}t.partition({c.str_lit(True) or repr('a')}), "
                     f"{p}t.rpartition('a'), {p}t.title(), {p}t.expandtabs({r.randint(0, 8)}), {p}t.isidentifier())"],
            # Nested format specs and alignment of non-ASCII text.
            lambda: [f"{p}w: int = {r.randint(0, 12)}",
                     f"{p}u: str = {c.str_lit()}",
                     "print(f'[{" + p + "u:>{" + p + "w}}][{" + p + "u:^{" + p + "w}.{" + p + "w}}][{3.14159:.{" + p + "w}f}]')"],
            # min/max/sorted with key and stability, enumerate with start.
            lambda: [f"{p}ws: list[str] = {sorted(r.sample(STR_ASCII + STR_UNI, 5))!r}",
                     f"print(min({p}ws, key=len), max({p}ws, key=len), sorted({p}ws, key=len), "
                     f"sorted({p}ws, key=lambda w: w[-1:]), list(enumerate({p}ws, {r.randint(-2, 3)}))[:2])"],
            # zip with strict=True raising, and str.join of a generator.
            lambda: [f"print('-'.join(str(i * i) for i in range({r.randint(0, 6)})))",
                     "try:",
                     f"{IND}print(list(zip([1, 2], [3{', 4' if r.random() < 0.5 else ''}], strict=True)))",
                     "except ValueError as e:",
                     f"{IND}print('ValueError', e)"],
            # Walrus in a loop condition and a comprehension.
            lambda: [f"{p}n: int = {r.randint(0, 50)}",
                     f"while ({p}m := {p}n // 3) > 1:",
                     f"{IND}print({p}m, end=' ')",
                     f"{IND}{p}n = {p}m",
                     "print()",
                     f"print([y for x in range(6) if (y := x * x) % 2 == 0])"],
            # Big ints through str/int round trips and the 4300-digit limit.
            lambda: [f"{p}big: int = {r.choice(INT_EDGE)} ** {r.randint(1, 40)}",
                     f"print(len(str({p}big)), int(str({p}big)) == {p}big, {p}big % 1000007, -{p}big // 7, {p}big.bit_length())"],
            # Augmented assignment on list items and dict values.
            lambda: [f"{p}xs: list[int] = {c.leaf(LINT, s)} + [0]",
                     f"{p}xs[-1] += {c.int_lit(True)}", f"{p}xs[0] *= {c.int_lit(True)}",
                     f"{p}cnt: dict[str, int] = {{}}",
                     f"for {p}ch in {c.str_lit()}:",
                     f"{IND}{p}cnt[{p}ch] = {p}cnt.get({p}ch, 0) + 1",
                     f"print({p}xs, {p}cnt)"],
        ]
        out: list[str] = []
        for make in r.sample(snippets, r.randint(1, 3)):
            out += make()
        return out

    def b_match_more(self, p: str) -> list[str]:
        r, c = self.c.r, self.c
        v = f"{p}v"
        if r.random() < 0.5:
            dicts = [f"{{'kind': {r.choice(['1', '2', repr('x')])}, 'n': {c.int_lit(True)}}}", "{'n': 5}", "{}",
                     f"{{'kind': 2, 'n': {c.int_lit(True)}, 'extra': 0}}"]
            return [f"{p}items: list[dict[str, int]] = [{', '.join(d for d in dicts if 'x' not in d)}]",
                    f"for {v} in {p}items:",
                    f"{IND}match {v}:",
                    f"{IND * 2}case {{'kind': 1, 'n': n}}:", f"{IND * 3}print('one', n)",
                    f"{IND * 2}case {{'kind': 2, **rest}}:", f"{IND * 3}print('two', rest)",
                    f"{IND * 2}case {{'n': n}} if n > 3:", f"{IND * 3}print('big n', n)",
                    f"{IND * 2}case _:", f"{IND * 3}print('other', len({v}))"]
        return [f"for {v} in [(1, 'a'), (2, 'b'), (0, ''), ({c.int_lit(True)}, {c.str_lit(True)})]:",
                f"{IND}match {v}:",
                f"{IND * 2}case (0, _):", f"{IND * 3}print('zero')",
                f"{IND * 2}case (n, 'a' | 'b') if n > 0:", f"{IND * 3}print('ab', n)",
                f"{IND * 2}case (n, s):", f"{IND * 3}print('pair', n, s[:2])"]
