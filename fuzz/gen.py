"""Program generator: a deterministic typed Python program per (seed, index).

`python3 fuzz/gen.py SEED [INDEX]` prints the program the fuzzer generates for
that case, so a finding's program can be regenerated from its seed alone.
"""

from __future__ import annotations

import dataclasses
import random
import sys

from gen_blocks import FEATURES, Blocks
from gen_core import LINT, STR, Core, Scope
from gen_stmts import IND, indent, risky

# Blocks that cannot move into a function body: they bind module globals
# (`global`) or are meant to be top-level statements.
NO_WRAP = {"global", "compute_top"}


@dataclasses.dataclass
class Block:
    feature: str
    lines: list[str]


@dataclasses.dataclass
class Program:
    header: list[str]
    blocks: list[Block]
    case: str

    def render(self, drop: frozenset[int] = frozenset()) -> tuple[str, list[tuple[int, int, int]]]:
        """The source and, per kept block, its (first line, last line, block index)."""
        lines = list(self.header)
        spans = []
        for i, b in enumerate(self.blocks):
            if i in drop:
                continue
            start = len(lines) + 1
            lines += b.lines
            spans.append((start, len(lines), i))
        return "\n".join(lines) + "\n", spans


def tail_block(c: Core, p: str) -> list[str]:
    """A last statement that (usually) raises an uncaught exception."""
    r = c.r
    s = Scope()
    s.prefix = p
    k = r.randrange(9)
    if k == 0:
        return [risky(c, s)]
    if k == 1:
        return [f"raise ValueError({c.expr(STR, s, 1)})"]
    if k == 2:
        return [f"raise KeyError({c.str_lit()})"]
    if k == 3:
        return [f"assert {c.expr('bool', s, 1)}, {c.str_lit()}"]
    if k == 4:
        return [f"raise SystemExit({r.choice(['0', '1', '3', '42', repr('bye')])})"]
    if k == 5:
        E = f"{p.upper()}Fail"
        return [f"class {E}(Exception):", f"{IND}pass", f"def {p}boom(n: int) -> int:",
                f"{IND}if n > {c.int_lit(True)}:", f"{IND * 2}raise {E}('n=' + str(n))", f"{IND}return n",
                f"print({p}boom({c.int_lit(True)}))", f"print({p}boom(100))"]
    if k == 6:
        return ["try:", f"{IND}{risky(c, s)}", "except Exception as e:",
                f"{IND}raise RuntimeError('during handling') from e"]
    if k == 7:
        return [f"{p}xs: list[int] = {c.leaf(LINT, s)}", f"print({p}xs.pop({c.int_lit(True)}))"]
    return [f"print(int({c.expr(STR, s, 1)}))"]


def generate(seed: int, index: int) -> Program:
    rng = random.Random(f"{seed}:{index}")
    c = Core(rng)
    bl = Blocks(c)
    feats = list(FEATURES)
    weights = [FEATURES[f] for f in feats]
    blocks = []
    for i in range(rng.randint(1, 4)):
        f = rng.choices(feats, weights)[0]
        p = f"b{i}_"
        lines = getattr(bl, "b_" + f)(p)
        if f not in NO_WRAP and rng.random() < 0.3:
            lines = [f"def {p}main() -> None:"] + indent(lines) + [f"{p}main()"]
            f += "+wrapped"
        blocks.append(Block(f, lines))
    if rng.random() < 0.2:
        blocks.append(Block("raise_tail", tail_block(c, "t_")))
    header = ["import math"] if "math" in c.uses else []
    header += sorted(bl.imports)
    return Program(header, blocks, f"{seed}:{index}")


def main() -> int:
    seed = int(sys.argv[1])
    index = int(sys.argv[2]) if len(sys.argv) > 2 else 0
    print(generate(seed, index).render()[0], end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())



