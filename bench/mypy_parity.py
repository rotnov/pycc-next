"""Strict mode refuses only what a type checker refuses (ROADMAP v0.9 accept 5).

Runs `pycc check` (strict, the default mode) and `mypy --strict` over the
corpus programs `bench/corpus.py` measures, and compares the two verdicts:

- every program pycc refuses must be rejected by mypy too, unless the
  "Strict refusals mypy accepts" table of docs/DEVIATIONS.md lists it with a
  reason; a listed program that pycc accepts or mypy rejects is a stale row
  and fails as well, so the table stays exact;
- a program mypy rejects and pycc accepts is reported as information (pycc
  infers what mypy --strict asks to be annotated).

Usage: python3 bench/mypy_parity.py [--pycc PATH] [--mypy "python3 -m mypy"] [-j 4]
Exit status 1 when the check fails.
"""
import argparse
import collections
import concurrent.futures
import os
import pathlib
import re
import shlex
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CORPUS = ROOT / "tests/corpus/codecontests/problems"
DEVIATIONS = ROOT / "docs/DEVIATIONS.md"
TABLE_HEADING = "## Strict refusals mypy accepts"

# mypy 2.x prints `path:line: error: message  [code]`; notes and the summary
# line are not errors.
MYPY_ERROR = re.compile(r"^(?P<file>[^:\n]+\.py):\d+(?::\d+)?: error: .*?(?:\s\s\[(?P<code>[\w-]+)\])?$", re.M)
PYCC_ERROR = re.compile(r"error\[(\w+)\]: ([^\n]{0,90})")


def problem_id(name: str) -> str:
    """The number a problem directory starts with (`049-1557-c-...` -> `049`)."""
    return name.split("-")[0]


def parse_mypy(output: str) -> dict[str, collections.Counter[str]]:
    """Per checked file stem (`p049`), the count of each mypy error code."""
    errors: dict[str, collections.Counter[str]] = collections.defaultdict(collections.Counter)
    for m in MYPY_ERROR.finditer(output):
        errors[pathlib.PurePath(m.group("file")).stem][m.group("code") or "?"] += 1
    return dict(errors)


def parse_allowed(markdown: str) -> dict[str, str]:
    """The `| problem | reason |` rows under TABLE_HEADING, by problem number."""
    allowed: dict[str, str] = {}
    lines = markdown.splitlines()
    try:
        start = lines.index(TABLE_HEADING) + 1
    except ValueError:
        return allowed
    for line in lines[start:]:
        if line.startswith("## "):
            break
        # A cell may hold an escaped `\|`.
        cells = [c.strip() for c in re.split(r"(?<!\\)\|", line.strip().strip("|"))]
        if len(cells) >= 2 and re.fullmatch(r"\d{3}", cells[0]):
            if not cells[-1]:
                raise ValueError(f"{TABLE_HEADING}: row {cells[0]} gives no reason")
            allowed[cells[0]] = cells[-1]
    return allowed


def compare(
    pycc_refused: dict[str, str],
    mypy_errors: dict[str, collections.Counter[str]],
    problems: list[str],
    allowed: dict[str, str],
) -> tuple[list[str], list[str]]:
    """Returns (failures, information) lines for the verdicts of `problems`.

    `pycc_refused` maps a refused problem number to pycc's first diagnostic;
    `mypy_errors` maps `p<number>` to its mypy error codes.
    """
    failures, info = [], []
    for pid in problems:
        refused = pid in pycc_refused
        rejected = bool(mypy_errors.get(f"p{pid}"))
        if refused and not rejected and pid not in allowed:
            failures.append(f"{pid}: pycc strict refuses ({pycc_refused[pid]}), mypy --strict accepts, "
                            f"and docs/DEVIATIONS.md does not list it")
        elif pid in allowed and not (refused and not rejected):
            state = ("pycc accepts it" if not refused else "mypy rejects it")
            failures.append(f"{pid}: listed in docs/DEVIATIONS.md, but {state}: remove the row")
        elif rejected and not refused:
            codes = ", ".join(f"{c} x{n}" if n > 1 else c for c, n in sorted(mypy_errors[f'p{pid}'].items()))
            info.append(f"{pid}: mypy rejects ({codes}), pycc strict accepts")
    for pid in sorted(set(allowed) - set(problems)):
        failures.append(f"{pid}: listed in docs/DEVIATIONS.md, but no such corpus program")
    return failures, info


def pycc_check(pycc: str, source: pathlib.Path) -> str | None:
    """pycc's first diagnostic when strict mode refuses `source`, else None."""
    r = subprocess.run([pycc, "check", str(source)], capture_output=True, text=True,
                       encoding="utf-8", errors="replace", timeout=300)
    if r.returncode == 0:
        return None
    m = PYCC_ERROR.search(r.stdout + r.stderr)
    return f"{m.group(1)}: {m.group(2)}" if m else (r.stderr or r.stdout).strip()[:120]


def run_mypy(mypy: list[str], sources: dict[str, pathlib.Path], tmp: pathlib.Path) -> str:
    """Runs `mypy --strict` once over every source, copied to `p<number>.py`
    (the programs are all `solution.py`), and returns its output."""
    src = tmp / "src"
    src.mkdir()
    for pid, path in sources.items():
        (src / f"p{pid}.py").write_bytes(path.read_bytes())
    r = subprocess.run([*mypy, "--strict", "--python-version", "3.13", "--no-incremental",
                        "--cache-dir", str(tmp / "cache"), "--no-error-summary", "--show-error-codes",
                        *sorted(str(p) for p in src.iterdir())],
                       capture_output=True, text=True, encoding="utf-8", errors="replace", cwd=tmp)
    # 0: no errors, 1: type errors; anything else is mypy failing to run.
    if r.returncode not in (0, 1):
        raise SystemExit(f"mypy failed (exit {r.returncode}):\n{r.stdout}{r.stderr}")
    return r.stdout


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pycc", default=str(ROOT / "target" / "release" /
                                          ("pycc.exe" if os.name == "nt" else "pycc")))
    ap.add_argument("--mypy", default=f"{shlex.quote(sys.executable)} -m mypy")
    ap.add_argument("-j", type=int, default=4)
    a = ap.parse_args()
    version = subprocess.run([*shlex.split(a.mypy), "--version"], capture_output=True, text=True)
    sources = {problem_id(p.name): p / "solution.py" for p in sorted(CORPUS.iterdir()) if p.is_dir()}
    problems = sorted(sources)
    with concurrent.futures.ThreadPoolExecutor(a.j) as ex:
        verdicts = dict(zip(problems, ex.map(lambda pid: pycc_check(a.pycc, sources[pid]), problems)))
    pycc_refused = {pid: d for pid, d in verdicts.items() if d is not None}
    with tempfile.TemporaryDirectory(prefix="pycc-mypy-") as tmp:
        mypy_errors = parse_mypy(run_mypy(shlex.split(a.mypy), sources, pathlib.Path(tmp)))
    allowed = parse_allowed(DEVIATIONS.read_text(encoding="utf-8"))
    failures, info = compare(pycc_refused, mypy_errors, problems, allowed)

    n = len(problems)
    rejected = sum(1 for pid in problems if mypy_errors.get(f"p{pid}"))
    print(f"mypy parity: {n} programs; {version.stdout.strip()}")
    print(f"  pycc strict refuses {len(pycc_refused)}, mypy --strict rejects {rejected}")
    print("  | program | pycc strict | mypy --strict |")
    print("  |---|---|---|")
    for pid in sorted(pycc_refused):
        codes = ", ".join(sorted(mypy_errors.get(f"p{pid}", {}))) or "accepts"
        note = " (listed in DEVIATIONS)" if pid in allowed else ""
        print(f"  | {pid} | {pycc_refused[pid]} | {codes}{note} |")
    print(f"information: {len(info)} programs mypy rejects and pycc strict accepts")
    for line in info:
        print(f"  {line}")
    for line in failures:
        print(f"FAIL {line}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
