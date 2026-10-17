"""Running one program under CPython and pycc's build modes, and comparing them.

An outcome is (stdout, exception, exit status). The exception is the text of the
last exception CPython's traceback reports (its type and message), with the
module prefix of a class defined in the program (`__main__.`, the extension
module's name) removed; frames are not compared (compiled code prints only the
last line, a documented deviation).
"""

from __future__ import annotations

import dataclasses
import os
import pathlib
import re
import subprocess
import sys

PYTHON = os.environ.get("PYCC_PYTHON", sys.executable)
ENV = {**os.environ, "PYTHONHASHSEED": "0", "PYTHONIOENCODING": "utf-8", "PYTHONDONTWRITEBYTECODE": "1"}

# Mode name -> `pycc build` flags. `ext*` modes build an extension module that a
# CPython driver imports.
MODES = {
    "strict": ["--release"],
    "nonstrict": ["--release", "--non-strict"],
    "ext": ["--ext"],
    "ext-nonstrict": ["--ext", "--non-strict"],
}

CRASH_MARKERS = ("panicked at", "LLVM ERROR", "internal compiler error", "RUST_BACKTRACE",
                 "Assertion `", "undefined reference", "Segmentation fault", "stack overflow",
                 "error: linking", "ld returned", "LLVM IR", "verifier")
DIAG = re.compile(r"^(?:error|warning)\[([A-Z]\d{4})\]: (.*)$", re.M)
DIAG_LOC = re.compile(r"-->\s+(\S+?):(\d+):(\d+)")


@dataclasses.dataclass
class Outcome:
    stdout: str
    exc: str
    status: int
    timeout: bool = False

    def describe(self) -> str:
        return f"status={self.status} exc={self.exc!r} timeout={self.timeout}"


@dataclasses.dataclass
class Build:
    ok: bool
    refused: bool  # a diagnostic: not a finding
    crash: bool  # a compiler crash: a finding
    text: str
    codes: list[tuple[str, str, int]]  # (code, message, line) of each error diagnostic
    seconds_out: bool = False


def last_exception(stderr: str, module: str) -> str:
    """The final exception's text from a traceback (or a bare message)."""
    text = stderr.replace("\r\n", "\n")
    marker = "Traceback (most recent call last):\n"
    if marker in text:
        text = text[text.rindex(marker) + len(marker):]
    lines = [ln for ln in text.split("\n") if not ln.startswith("  ")]
    out = "\n".join(lines).strip("\n")
    for prefix in ("__main__.", module + "."):
        out = re.sub(r"^" + re.escape(prefix), "", out, flags=re.M)
    return out


def run_process(cmd: list[str], cwd: pathlib.Path, timeout: float, module: str) -> Outcome:
    try:
        r = subprocess.run(cmd, cwd=cwd, capture_output=True, timeout=timeout, env=ENV)
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"").decode("utf-8", "replace")
        return Outcome(out, "", -1, timeout=True)
    out = r.stdout.decode("utf-8", "replace")
    err = r.stderr.decode("utf-8", "replace")
    return Outcome(out, last_exception(err, module) if r.returncode else "", r.returncode)


def run_cpython(src: pathlib.Path, timeout: float = 10) -> Outcome:
    return run_process([PYTHON, src.name], src.parent, timeout, "__main__")


DRIVER = "import sys\nsys.path.insert(0, '.')\nimport {mod}\n"


def build(pycc: str, src: pathlib.Path, out: pathlib.Path, mode: str, timeout: float = 120) -> Build:
    cmd = [pycc, "build", str(src), "-o", str(out), *MODES[mode]]
    try:
        r = subprocess.run(cmd, capture_output=True, timeout=timeout, env=ENV)
    except subprocess.TimeoutExpired:
        return Build(False, False, True, f"build timed out after {timeout}s", [], seconds_out=True)
    text = (r.stdout + r.stderr).decode("utf-8", "replace")
    if r.returncode == 0:
        return Build(True, False, False, text, [])
    codes = []
    for m in DIAG.finditer(text):
        if not m.group(0).startswith("error"):
            continue
        loc = DIAG_LOC.search(text, m.end())
        line = int(loc.group(2)) if loc and pathlib.Path(loc.group(1)).name == src.name else 0
        codes.append((m.group(1), m.group(2), line))
    crash = r.returncode < 0 or r.returncode == 101 or not codes or any(k in text for k in CRASH_MARKERS)
    return Build(False, not crash, crash, text, codes)


def run_built(out: pathlib.Path, mode: str, timeout: float = 20) -> Outcome:
    if mode.startswith("ext"):
        return run_process([PYTHON, "-c", DRIVER.format(mod=out.name)], out.parent, timeout, out.name)
    return run_process([str(out)], out.parent, timeout, out.name)


def clean_build(out: pathlib.Path) -> None:
    for p in out.parent.glob(out.name + "*"):
        if p.is_dir():
            import shutil

            shutil.rmtree(p, ignore_errors=True)
        else:
            p.unlink(missing_ok=True)


# An object's address (`<function f at 0x7f...>`) differs between any two runs.
ADDRESS = re.compile(r"\b0x[0-9a-f]{6,16}\b")


def compare(want: Outcome, got: Outcome) -> str | None:
    """The kind of mismatch, or None when the outcomes agree."""
    if got.timeout:
        return "timeout"
    if got.status < 0 or got.status in (134, 139):
        return "crash"
    if ADDRESS.sub("0x?", want.stdout) != ADDRESS.sub("0x?", got.stdout):
        return "stdout"
    if want.status != got.status:
        return "status"
    if want.exc != got.exc:
        return "exception"
    return None


# Differences docs/DEVIATIONS.md documents, recognized by what pycc raised:
# such a mismatch is counted (and listed in the summary), not a finding. The
# generator avoids these shapes where it can; this is the safety net.
KNOWN_DEVIATIONS = [
    (re.compile(r"storing a bigint-valued `int` in a container is not supported yet"),
     "an int beyond 63 bits stored in a container or a boxed tuple"),
    (re.compile(r"in an `int \| float` container is not supported yet"),
     "an int | float float beyond the inline range stored in a container"),
    (re.compile(r"'builtins\.generator' object has no attribute"),
     "a generator crossing an --ext boundary is an iterator only (fallback code cannot send/throw)"),
]


def known_deviation(want: Outcome, got: Outcome) -> str | None:
    if got.exc and got.exc != want.exc:
        for pattern, label in KNOWN_DEVIATIONS:
            if pattern.search(got.exc):
                return label
    return None


# CPython outcomes that mean the generator produced an invalid program (a bug in
# the fuzzer, not in pycc): such a program is discarded.
GENERATOR_BUGS = ("TypeError", "NameError", "UnboundLocalError", "AttributeError", "SyntaxError",
                  "RecursionError", "IndentationError", "MemoryError", "ImportError", "ModuleNotFoundError")


def generator_bug(o: Outcome) -> bool:
    head = o.exc.split(":", 1)[0].strip()
    return o.timeout or head in GENERATOR_BUGS
