#!/usr/bin/env python3
"""Differential fuzzer: generated typed programs under CPython 3.13 and pycc.

Each case is a program `gen.generate(seed, index)` produces. It runs under
CPython (the oracle) and is built by pycc in each mode (`harness.MODES`): native
strict, native `--non-strict`, `--ext` (imported by a CPython driver) and, for a
share of the cases, `--ext --non-strict`. A mode compares stdout, the uncaught
exception (type and message) and the exit status with CPython's.

A diagnostic is not a mismatch: the block the diagnostic points at is dropped
(and counted as refused for its feature) and the rest of the program is tried
again, so refusal rates per feature and mode are visible in the summary. A
compiler crash (panic, LLVM error, link failure, a build with no diagnostic) is
a finding, as is every mismatch; each finding is reduced to a minimal program
(`reduce.py`) and written to the findings directory.

    python3 fuzz/fuzz.py --seconds 120 -j 4 --seed 1
    python3 fuzz/fuzz.py --replay 1:57          # rerun one case verbosely

Exit status 1 when there is any finding.
"""

from __future__ import annotations

import argparse
import ast
import collections
import concurrent.futures
import dataclasses
import hashlib
import json
import os
import pathlib
import random
import re
import shutil
import sys
import threading
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import harness  # noqa: E402
from gen import Block, Program, generate  # noqa: E402
from reduce import reduce  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent


@dataclasses.dataclass
class Finding:
    case: str
    mode: str
    kind: str
    source: str
    want: str
    got: str
    detail: str
    signature: str
    reduced: str | None = None
    reduce_tests: int = 0
    reduced_outcomes: dict[str, str] | None = None
    duplicate_of: str | None = None


class Stats:
    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.cases = 0
        self.discarded = collections.Counter()
        self.raising = 0
        self.tried = collections.Counter()  # (mode, feature) -> blocks offered
        self.refused = collections.Counter()  # (mode, feature) -> blocks refused
        self.codes = collections.Counter()  # (mode, feature, code: message) -> count
        self.compared = collections.Counter()  # mode -> programs compared
        self.reductions = 0
        self.pruned = collections.Counter()  # mode -> refused statements dropped
        self.known = collections.Counter()  # documented deviation -> mismatches it explains
        self.findings: list[Finding] = []
        self.signatures: dict[str, str] = {}


def mask(text: str) -> str:
    return re.sub(r"0x[0-9a-f]+|\d+", "N", text)[:160]


def signature(mode: str, kind: str, want: harness.Outcome, got: harness.Outcome, detail: str) -> str:
    if kind == "compiler-crash":
        m = re.search(r"panicked at [^\n]*\n([^\n]*)", detail) or re.search(r"(LLVM ERROR[^\n]*|error[^\n]*)", detail)
        body = mask(m.group(1) if m else detail[:120])
    elif got.exc and got.exc != want.exc:
        # pycc raised where CPython did not (or raised something else): the
        # exception names the bug better than the line where output stopped.
        body = mask(f"raised {got.exc.splitlines()[-1] if got.exc else ''} (CPython: {want.exc[:60]})")
    elif kind == "stdout":
        w, g = want.stdout.splitlines(), got.stdout.splitlines()
        i = next((i for i, (a, b) in enumerate(zip(w, g)) if a != b), min(len(w), len(g)))
        body = mask((w[i] if i < len(w) else "<eof>") + " => " + (g[i] if i < len(g) else "<eof>"))
    elif kind in ("exception", "status"):
        body = mask(f"{want.exc}|{want.status} => {got.exc}|{got.status}")
    else:
        body = ""
    # Mode-agnostic: one bug seen in several modes is reduced once.
    return f"{kind}:{body}"


class Case:
    """One generated program through every mode."""

    def __init__(self, cfg: argparse.Namespace, prog: Program, work: pathlib.Path, stats: Stats, rng: random.Random) -> None:
        self.cfg, self.prog, self.work, self.stats = cfg, prog, work, stats
        self.rng = rng
        self.oracle: dict[str, harness.Outcome] = {}
        self.stem = "p" + hashlib.sha1(prog.case.encode()).hexdigest()[:10]

    def log(self, msg: str) -> None:
        if self.cfg.verbose:
            print(f"[{self.prog.case}] {msg}", flush=True)

    def cpython(self, src: str) -> harness.Outcome:
        if src not in self.oracle:
            path = self.work / f"{self.stem}.py"
            path.write_text(src)
            self.oracle[src] = harness.run_cpython(path, self.cfg.timeout)
        return self.oracle[src]

    def build(self, src: str, mode: str) -> tuple[harness.Build, pathlib.Path]:
        path = self.work / f"{self.stem}.py"
        path.write_text(src)
        out = self.work / (self.stem + "_" + mode.replace("-", "_"))
        harness.clean_build(out)
        return harness.build(self.cfg.pycc, path, out, mode), out

    def run(self) -> None:
        src, _ = self.prog.render()
        want = self.cpython(src)
        st = self.stats
        if harness.generator_bug(want):
            with st.lock:
                st.discarded["timeout" if want.timeout else want.exc.split(":", 1)[0]] += 1
            self.log(f"discarded: {want.describe()}")
            if self.cfg.verbose:
                print(src)
            return
        with st.lock:
            st.cases += 1
            st.raising += bool(want.exc)
        modes = [m for m in self.cfg.modes if m != "ext-nonstrict" or self.rng.random() < self.cfg.ext_nonstrict_share]
        for mode in modes:
            self.run_mode(mode)

    def run_mode(self, mode: str) -> None:
        st = self.stats
        src, spans = self.prog.render()
        refused_blocks: set[int] = set()
        with st.lock:
            for b in self.prog.blocks:
                st.tried[(mode, b.feature)] += 1
        for _ in range(MAX_PRUNES):
            text = src.split("\n")
            if not any(ln.strip() for lo, hi, _ in spans for ln in text[lo - 1:hi]):
                return
            b, out = self.build(src, mode)
            if b.crash:
                kind = "build-timeout" if b.seconds_out else "compiler-crash"
                self.finding(mode, kind, src, harness.Outcome("", "", 0), harness.Outcome("", "", 0), b.text)
                harness.clean_build(out)
                return
            if b.refused:
                lines = [ln for _, _, ln in b.codes]
                hit = {i for lo, hi, i in spans for ln in lines if lo <= ln <= hi}
                if not hit:
                    # A diagnostic outside every block (an import line): give up on this mode.
                    with st.lock:
                        for code, msg, _ in b.codes[:1]:
                            st.codes[(mode, "header", f"{code}: {msg[:70]}")] += 1
                    return
                with st.lock:
                    for i in hit - refused_blocks:
                        st.refused[(mode, self.prog.blocks[i].feature)] += 1
                    for code, msg, ln in b.codes:
                        feat = next((self.prog.blocks[i].feature for lo, hi, i in spans if lo <= ln <= hi), "header")
                        st.codes[(mode, feat.split("+")[0], f"{code}: {re.sub(r'`[^`]{24,}`', '`...`', msg)[:90]}")] += 1
                refused_blocks |= hit
                # First drop only the statements the diagnostics point at, so
                # the rest of the block is still compared; when that leaves a
                # program CPython rejects, drop the whole blocks.
                pruned = prune_statements(src, lines)
                if pruned is not None and not harness.generator_bug(self.cpython(pruned)):
                    with st.lock:
                        st.pruned[mode] += 1
                    self.log(f"{mode}: pruned statements at {lines}: {[(c, m[:100], ln) for c, m, ln in b.codes[:2]]}")
                    src = pruned
                    continue
                src = blank_lines(src, [(lo, hi) for lo, hi, i in spans if i in hit])
                self.log(f"{mode}: refused blocks {sorted(hit)}: {[(c, m[:100], ln) for c, m, ln in b.codes[:2]]}")
                continue
            want = self.cpython(src)
            if harness.generator_bug(want):
                return
            got = harness.run_built(out, mode, self.cfg.timeout * 2)
            harness.clean_build(out)
            with st.lock:
                st.compared[mode] += 1
            kind = harness.compare(want, got)
            if kind:
                self.finding(mode, kind, src, want, got, "")
            else:
                self.log(f"{mode}: ok")
            return

    # ------------------------------------------------------------ findings
    def finding(self, mode: str, kind: str, src: str, want: harness.Outcome, got: harness.Outcome, detail: str) -> None:
        known = harness.known_deviation(want, got)
        if known:
            with self.stats.lock:
                self.stats.known[known] += 1
            self.log(f"{mode}: known deviation: {known}")
            return
        sig = signature(mode, kind, want, got, detail)
        f = Finding(self.prog.case, mode, kind, src, want.describe() + "\n" + want.stdout,
                    got.describe() + "\n" + got.stdout, detail[-4000:], sig)
        with self.stats.lock:
            dup = self.stats.signatures.get(sig)
            if dup is None:
                self.stats.signatures[sig] = f.case
            f.duplicate_of = dup
            self.stats.findings.append(f)
        print(f"FINDING [{f.case}] {mode} {kind}" + (f" (same as {dup})" if dup else "") + f": {sig}", flush=True)
        budget = self.reduce_budget() if dup is None else 0
        if budget > 0:
            pred = self.predicate(mode, kind, sig)
            if pred(src):
                f.reduced, f.reduce_tests = reduce(src, pred, budget)
                print(f"  reduced [{f.case}] {len(src)} -> {len(f.reduced)} bytes in {f.reduce_tests} tests", flush=True)
                f.reduced_outcomes = self.outcomes(f.reduced, mode)
            else:
                f.detail += "\n(did not reproduce on a second run: flaky)"
        write_finding(self.cfg.findings, f)

    def reduce_budget(self) -> float:
        """Seconds to reduce one finding: `--reduce-seconds`, at most
        `--max-reductions` of them, ending by `--reduce-grace` after the run."""
        cfg = self.cfg
        with self.stats.lock:
            if cfg.reduce_seconds <= 0 or self.stats.reductions >= cfg.max_reductions:
                return 0
            self.stats.reductions += 1
        return max(0.0, min(cfg.reduce_seconds, cfg.deadline + cfg.reduce_grace - time.monotonic()))

    def outcomes(self, src: str, mode: str) -> dict[str, str]:
        """CPython's and the mode's outcome for `src`, as text."""
        want = self.cpython(src)
        b, out = self.build(src, mode)
        if not b.ok:
            harness.clean_build(out)
            return {"cpython": want.describe() + "\n" + want.stdout, "pycc": b.text[-4000:]}
        got = harness.run_built(out, mode, self.cfg.timeout * 2)
        harness.clean_build(out)
        return {"cpython": want.describe() + "\n" + want.stdout, "pycc": got.describe() + "\n" + got.stdout}

    def predicate(self, mode: str, kind: str, sig: str):
        def still_fails(src: str) -> bool:
            want = self.cpython(src)
            if harness.generator_bug(want):
                return False
            b, out = self.build(src, mode)
            try:
                if kind in ("compiler-crash", "build-timeout"):
                    return b.crash and signature(mode, kind, want, want, b.text) == sig
                if not b.ok:
                    return False
                got = harness.run_built(out, mode, self.cfg.timeout * 2)
            finally:
                harness.clean_build(out)
            k = harness.compare(want, got)
            if k != kind:
                return False
            # Keep the same kind of difference; the text may shrink with the
            # program, but an exception pycc raised must stay the same one
            # (a reduction may otherwise drift into another, type-invalid
            # shape that fails differently).
            if ":raised " in sig:
                return signature(mode, kind, want, got, "") == sig
            return True
        return still_fails


# Refusals pruned (statements or blocks) before a mode gives up on a program.
MAX_PRUNES = 16


def blank_lines(src: str, ranges: list[tuple[int, int]]) -> str:
    """`src` with the 1-based inclusive line `ranges` emptied (line numbers kept)."""
    lines = src.split("\n")
    for lo, hi in ranges:
        for k in range(lo - 1, min(hi, len(lines))):
            lines[k] = ""
    return "\n".join(lines)


def prune_statements(src: str, lines: list[int]) -> str | None:
    """`src` with the innermost statement holding each of `lines` replaced by
    `pass` (and blank lines, so line numbers stay put); None when a line is
    in no statement but the module itself or nothing changes."""
    try:
        tree = ast.parse(src)
    except SyntaxError:
        return None
    stmts = [n for n in ast.walk(tree) if isinstance(n, ast.stmt)]
    text = src.split("\n")
    out = list(text)
    for line in lines:
        best = None
        for s in stmts:
            first = min([s.lineno] + [d.lineno for d in getattr(s, "decorator_list", [])])
            if first <= line <= (s.end_lineno or s.lineno):
                if best is None or (s.end_lineno or 0) - first < (best[1] - best[0]):
                    best = (first, s.end_lineno or s.lineno, s.col_offset)
        if best is None:
            return None
        lo, hi, col = best
        if all(not out[k].strip() for k in range(lo - 1, hi)):
            continue
        out[lo - 1] = " " * col + "pass"
        for k in range(lo, hi):
            out[k] = ""
    pruned = "\n".join(out)
    return pruned if pruned != src else None


def write_finding(root: pathlib.Path, f: Finding) -> None:
    d = root / (f.case.replace(":", "-") + "-" + f.mode)
    d.mkdir(parents=True, exist_ok=True)
    (d / "original.py").write_text(f.source)
    if f.reduced:
        (d / "reduced.py").write_text(f.reduced)
    flags = " ".join(harness.MODES[f.mode])
    info = {
        "case": f.case, "mode": f.mode, "kind": f.kind, "signature": f.signature,
        "duplicate_of": f.duplicate_of, "reduce_tests": f.reduce_tests,
        "build": f"pycc build reduced.py -o prog {flags}",
        "reduced": f.reduced_outcomes, "cpython": f.want, "pycc": f.got, "detail": f.detail,
    }
    (d / "finding.json").write_text(json.dumps(info, indent=2, ensure_ascii=False) + "\n")


def summary(st: Stats, cfg: argparse.Namespace, elapsed: float) -> str:
    out = [f"fuzz: {st.cases} programs compared in {elapsed:.0f}s (seed {cfg.seed}, -j {cfg.jobs}); "
           f"{st.raising} raise an uncaught exception; discarded {sum(st.discarded.values())} {dict(st.discarded)}"]
    out.append("compared per mode: " + ", ".join(f"{m} {st.compared[m]}" for m in cfg.modes))
    feats = sorted({f.split("+")[0] for _, f in st.tried})
    out.append("refused statements dropped (the rest of the block still compared): "
               + ", ".join(f"{m} {st.pruned[m]}" for m in cfg.modes))
    out.append("refusal rate per feature (blocks with a refusal / tried blocks):")
    out.append(f"  {'feature':16}" + "".join(f"{m:>16}" for m in cfg.modes))
    for feat in feats:
        row = f"  {feat:16}"
        for m in cfg.modes:
            t = sum(v for (mm, f), v in st.tried.items() if mm == m and f.split("+")[0] == feat)
            r = sum(v for (mm, f), v in st.refused.items() if mm == m and f.split("+")[0] == feat)
            row += f"{f'{r}/{t}':>10} {100 * r / t if t else 0:3.0f}%"
        out.append(row)
    out.append("top refusals (mode, feature, diagnostic):")
    for (m, f, c), n in st.codes.most_common(cfg.top_refusals):
        out.append(f"  {n:5}  {m:13} {f:16} {c}")
    if st.known:
        out.append("documented deviations seen (not findings):")
        for label, n in st.known.most_common():
            out.append(f"  {n:5}  {label}")
    uniq = [f for f in st.findings if not f.duplicate_of]
    out.append(f"findings: {len(st.findings)} ({len(uniq)} distinct)")
    for f in uniq:
        dups = sum(1 for g in st.findings if g.duplicate_of == f.case and g.signature == f.signature)
        out.append(f"  [{f.case}] {f.mode} {f.kind} (+{dups} more): {f.signature}")
    return "\n".join(out)


def worker(cfg: argparse.Namespace, stats: Stats, deadline: float, counter: list[int], wid: int) -> None:
    work = cfg.work / f"w{wid}"
    work.mkdir(parents=True, exist_ok=True)
    while time.monotonic() < deadline:
        with stats.lock:
            index = counter[0]
            counter[0] += 1
        if cfg.count and index >= cfg.count:
            return
        prog = generate(cfg.seed, index)
        try:
            Case(cfg, prog, work, stats, random.Random(f"modes:{cfg.seed}:{index}")).run()
        except Exception as e:  # noqa: BLE001 - a harness error must not stop the run
            print(f"harness error on case {prog.case}: {e!r}", flush=True)
            with stats.lock:
                stats.discarded["harness-error"] += 1
        for p in work.iterdir():
            if p.is_dir():
                shutil.rmtree(p, ignore_errors=True)
            else:
                p.unlink(missing_ok=True)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--seconds", type=float, default=60, help="stop starting new cases after this long")
    ap.add_argument("--count", type=int, default=0, help="stop after this many cases (0: no limit)")
    ap.add_argument("--seed", type=int, default=None, help="run seed (default: from the clock)")
    ap.add_argument("-j", "--jobs", type=int, default=max(1, (os.cpu_count() or 2) - 1))
    ap.add_argument("--pycc", default=str(ROOT / "target/release/pycc"))
    ap.add_argument("--findings", type=pathlib.Path, default=ROOT / "fuzz/findings")
    ap.add_argument("--work", type=pathlib.Path, default=ROOT / "fuzz/.work")
    ap.add_argument("--modes", default="strict,nonstrict,ext,ext-nonstrict")
    ap.add_argument("--ext-nonstrict-share", type=float, default=0.25,
                    help="share of the cases also built as --ext --non-strict")
    ap.add_argument("--timeout", type=float, default=10, help="CPython run timeout (compiled: twice this)")
    ap.add_argument("--reduce-seconds", type=float, default=300, help="time budget to reduce one finding (0: off)")
    ap.add_argument("--max-reductions", type=int, default=12, help="distinct findings reduced per run")
    ap.add_argument("--reduce-grace", type=float, default=900,
                    help="reductions end this many seconds after --seconds at the latest")
    ap.add_argument("--replay", help="SEED:INDEX: run one case verbosely")
    ap.add_argument("--program", type=pathlib.Path,
                    help="run (and reduce) this program file instead of generated ones, verbosely")
    ap.add_argument("--top-refusals", type=int, default=25)
    ap.add_argument("--summary-json", type=pathlib.Path, help="also write the statistics as JSON here")
    ap.add_argument("-v", "--verbose", action="store_true")
    cfg = ap.parse_args()
    cfg.modes = cfg.modes.split(",")
    if cfg.replay:
        cfg.seed, idx = map(int, cfg.replay.split(":"))
        cfg.count, cfg.verbose, cfg.jobs = idx + 1, True, 1
    if cfg.program and cfg.seed is None:
        cfg.seed = 0
    if cfg.seed is None:
        cfg.seed = int(time.time()) % 1_000_000
    cfg.findings = cfg.findings.resolve()
    cfg.work = cfg.work.resolve() / f"run-{os.getpid()}"
    cfg.work.mkdir(parents=True, exist_ok=True)
    stats = Stats()
    start = time.monotonic()
    cfg.deadline = start + cfg.seconds
    if cfg.program:
        cfg.seed, cfg.verbose, cfg.jobs = 0, True, 1
        lines = cfg.program.read_text().rstrip("\n").split("\n")
        prog = Program([], [Block("program", lines)], f"file-{cfg.program.stem}")
        Case(cfg, prog, cfg.work, stats, random.Random(0)).run()
    elif cfg.replay:
        prog = generate(cfg.seed, idx)
        print(prog.render()[0])
        Case(cfg, prog, cfg.work, stats, random.Random(f"modes:{cfg.seed}:{idx}")).run()
    else:
        counter = [0]
        deadline = cfg.deadline
        with concurrent.futures.ThreadPoolExecutor(cfg.jobs) as ex:
            futs = [ex.submit(worker, cfg, stats, deadline, counter, w) for w in range(cfg.jobs)]
            for fu in futs:
                fu.result()
    shutil.rmtree(cfg.work, ignore_errors=True)
    text = summary(stats, cfg, time.monotonic() - start)
    print(text)
    if cfg.findings.exists() or stats.findings:
        cfg.findings.mkdir(parents=True, exist_ok=True)
        (cfg.findings / "SUMMARY.txt").write_text(text + "\n")
    if cfg.summary_json:
        cfg.summary_json.parent.mkdir(parents=True, exist_ok=True)
        cfg.summary_json.write_text(json.dumps({
            "cases": stats.cases, "raising": stats.raising, "compared": stats.compared,
            "discarded": stats.discarded,
            "refused": {f"{m}/{f}": [stats.refused[(m, f)], t] for (m, f), t in stats.tried.items()},
            "findings": [dataclasses.asdict(f) for f in stats.findings],
        }, indent=2, ensure_ascii=False))
    return 1 if stats.findings else 0


if __name__ == "__main__":
    sys.exit(main())
