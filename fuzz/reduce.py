"""Delta-debugging reducer over the Python AST.

`reduce(src, still_fails, deadline)` shrinks `src` while `still_fails(candidate)`
holds: it deletes statement chunks (ddmin over every statement list), unwraps
compound statements into their bodies, and replaces expressions by one of their
sub-expressions or by a small constant, repeating until a full pass changes
nothing or the deadline passes. The predicate decides validity: a candidate
CPython rejects or pycc refuses simply does not fail the same way.
"""

from __future__ import annotations

import ast
import copy
import time
import warnings
from collections.abc import Callable

Path = tuple[tuple[str, int], ...]  # (field name, index or -1) steps from the module

STMT_FIELDS = ("body", "orelse", "finalbody")
MUST_HAVE_BODY = {"body"}


def get(tree: ast.AST, path: Path) -> object:
    node: object = tree
    for field, idx in path:
        node = getattr(node, field)
        if idx >= 0:
            node = node[idx]  # type: ignore[index]
    return node


def stmt_lists(tree: ast.AST) -> list[Path]:
    """Paths to every statement list, outermost first."""
    out: list[Path] = []

    def walk(node: ast.AST, path: Path) -> None:
        for field in STMT_FIELDS:
            stmts = getattr(node, field, None)
            if isinstance(stmts, list) and stmts and isinstance(stmts[0], ast.stmt):
                out.append(path + ((field, -1),))
                for i, s in enumerate(stmts):
                    walk(s, path + ((field, i),))
        for field in ("handlers", "cases"):
            for i, h in enumerate(getattr(node, field, None) or []):
                walk(h, path + ((field, i),))

    walk(tree, ())
    return out


def expr_paths(tree: ast.AST) -> list[Path]:
    """Paths to every expression node, outermost first (breadth first)."""
    out: list[Path] = []
    queue: list[tuple[ast.AST, Path]] = [(tree, ())]
    while queue:
        node, path = queue.pop(0)
        for field, value in ast.iter_fields(node):
            if isinstance(value, list):
                for i, v in enumerate(value):
                    if isinstance(v, ast.AST):
                        p = path + ((field, i),)
                        if isinstance(v, ast.expr):
                            out.append(p)
                        queue.append((v, p))
            elif isinstance(value, ast.AST):
                p = path + ((field, -1),)
                if isinstance(value, ast.expr):
                    out.append(p)
                queue.append((value, p))
    return out


def replace_at(tree: ast.AST, path: Path, new: object) -> None:
    parent = get(tree, path[:-1])
    field, idx = path[-1]
    if idx >= 0:
        getattr(parent, field)[idx] = new
    else:
        setattr(parent, field, new)


def unparse(tree: ast.AST) -> str | None:
    try:
        src = ast.unparse(ast.fix_missing_locations(tree)) + "\n"
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            compile(src, "<reduced>", "exec")
        return src
    except (SyntaxError, ValueError, TypeError, AttributeError, RecursionError):
        return None


class Reducer:
    def __init__(self, src: str, still_fails: Callable[[str], bool], deadline: float) -> None:
        self.tree = ast.parse(src)
        self.best = src
        self.still_fails = still_fails
        self.deadline = deadline
        self.tests = 0
        self.seen: set[str] = {src}

    def out_of_time(self) -> bool:
        return time.monotonic() > self.deadline

    def attempt(self, tree: ast.AST) -> bool:
        src = unparse(tree)
        if src is None or src in self.seen or len(src) >= len(self.best):
            return False
        self.seen.add(src)
        self.tests += 1
        if self.still_fails(src):
            self.tree, self.best = tree, src
            return True
        return False

    def pass_statements(self) -> bool:
        changed = False
        i = 0
        while i < len(paths := stmt_lists(self.tree)) and not self.out_of_time():
            path = paths[i]
            stmts = get(self.tree, path)
            n = len(stmts)  # type: ignore[arg-type]
            chunk = max(n // 2, 1)
            while chunk >= 1 and not self.out_of_time():
                start = 0
                progressed = False
                while start < len(get(self.tree, path)) and not self.out_of_time():  # type: ignore[arg-type]
                    cand = copy.deepcopy(self.tree)
                    lst = get(cand, path)
                    del lst[start:start + chunk]  # type: ignore[index, attr-defined]
                    if not lst and path[-1][0] in MUST_HAVE_BODY:
                        lst.append(ast.Pass())  # type: ignore[attr-defined]
                    if self.attempt(cand):
                        changed = progressed = True
                    else:
                        start += chunk
                if not progressed:
                    chunk //= 2
            i += 1
        return changed

    def pass_unwrap(self) -> bool:
        """Replace a compound statement by its body (or else branch)."""
        changed = False
        for path in stmt_lists(self.tree):
            if self.out_of_time():
                break
            stmts = get(self.tree, path)
            for idx in range(len(stmts) - 1, -1, -1):  # type: ignore[arg-type]
                node = stmts[idx]  # type: ignore[index]
                for field in ("body", "orelse", "finalbody"):
                    inner = getattr(node, field, None)
                    if not isinstance(inner, list) or not inner or isinstance(node, (ast.FunctionDef, ast.ClassDef)):
                        continue
                    cand = copy.deepcopy(self.tree)
                    lst = get(cand, path)
                    lst[idx:idx + 1] = copy.deepcopy(inner)  # type: ignore[index]
                    if self.attempt(cand):
                        changed = True
                        break
                else:
                    continue
                break
        return changed

    def pass_expressions(self) -> bool:
        changed = False
        i = 0
        while i < len(paths := expr_paths(self.tree)) and not self.out_of_time():
            path = paths[i]
            node = get(self.tree, path)
            options: list[ast.expr] = [c for c in ast.iter_child_nodes(node) if isinstance(c, ast.expr)]  # type: ignore[arg-type]
            if not isinstance(node, (ast.Constant, ast.Name)):
                options += [ast.Constant(0), ast.Constant(1), ast.Constant(""), ast.Constant(0.0), ast.Constant(True)]
            elif isinstance(node, ast.Constant) and isinstance(node.value, str) and len(node.value) > 1:
                options += [ast.Constant(node.value[:len(node.value) // 2]), ast.Constant(node.value[:1])]
            elif isinstance(node, ast.Constant) and isinstance(node.value, int) and not isinstance(node.value, bool) and abs(node.value) > 1:
                options += [ast.Constant(0), ast.Constant(1), ast.Constant(node.value // 2)]
            for opt in options:
                cand = copy.deepcopy(self.tree)
                try:
                    replace_at(cand, path, copy.deepcopy(opt))
                except (AttributeError, IndexError, TypeError):
                    continue
                if self.attempt(cand):
                    changed = True
                    break
            i += 1
        return changed

    def run(self) -> str:
        while not self.out_of_time():
            changed = self.pass_statements()
            changed |= self.pass_unwrap()
            changed |= self.pass_expressions()
            if not changed:
                break
        return self.best


def reduce(src: str, still_fails: Callable[[str], bool], seconds: float) -> tuple[str, int]:
    r = Reducer(src, still_fails, time.monotonic() + seconds)
    return r.run(), r.tests
