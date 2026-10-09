//! #1470: an instance of a regular compiled class is a subscript key, a
//! rich-comparison operand and a list-display element opposite a CPython
//! object, crossing as its `PyccExtInstance` carrier, whose type compares
//! and hashes through the class's own dunders (#1427) in both artifacts.
//! Before this change all three positions were refused with `I0404`,
//! because an embedded executable's carriers had no slots.
//!
//! [`DRIVER`] runs against the `--ext` artifact of [`MODULE`] and against
//! the same source imported as plain Python, so CPython is the oracle for
//! every line: `d[inst]` against a host dict whose key is an equal but
//! distinct object, `inst == host_obj` and `host_obj == inst` decided only by
//! `__eq__`, the derived `!=`, a list display observed through `==`,
//! `count`, `in` and `index` (never printed: a carrier's `repr` names
//! its address), and a class with no dunder keyed and compared by identity,
//! the same instance crossing twice as the same object.
//!
//! [`EMBEDDED`] is the same behaviour in an embedded executable, compared
//! byte for byte with CPython 3.14.7. A list display reaches an object slot
//! there only through an `object` annotation, which a native program cannot
//! write, so that position is pinned by the `--ext` half alone.
//!
//! The hosted tests are `#[ignore]`d for the reason every `ext` and embed
//! test is; [`an_uninstallable_slot_is_refused_before_the_embed_probe`] needs
//! no interpreter and runs everywhere.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

fn run(script: &str, path_entry: &Path, cwd: &Path) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .env("PYTHONPATH", path_entry)
        .current_dir(cwd)
        .output()
        .expect("python3 should spawn")
}

/// `K` compares and hashes by value; `P` defines no dunder.
const MODULE: &str = r#"from typing import Any


class K:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other: Any) -> Any:
        if not isinstance(other, K):
            return NotImplemented
        return self.v == other.v

    def __hash__(self) -> int:
        return self.v


class P:
    def __init__(self, v: int) -> None:
        self.v = v


def lookup(d: Any, v: int) -> Any:
    return d[K(v)]


def lookup_plain(d: Any, v: int) -> Any:
    return d.get(P(v), "missing")


def eq_left(o: Any, v: int) -> Any:
    return K(v) == o


def eq_right(o: Any, v: int) -> Any:
    return o == K(v)


def ne_left(o: Any, v: int) -> Any:
    return K(v) != o


def plain_eq(o: Any, v: int) -> Any:
    return P(v) == o


def as_list(v: int) -> Any:
    x: object = [K(v), 7, K(v + 1)]
    return x


def same(v: int) -> Any:
    p = P(v)
    x: object = [p, p, P(v)]
    return x
"#;

const DRIVER: &str = r#"import keyed as m

d = {m.K(1): "one", m.K(2): "two"}
print(m.lookup(d, 1), m.lookup(d, 2))
try:
    m.lookup(d, 3)
except KeyError as e:
    print("KeyError", type(e.args[0]).__name__)
print(m.lookup_plain({m.P(1): 1}, 1))
print(m.eq_left(m.K(4), 4), m.eq_left(m.K(4), 5), m.eq_left(None, 4))
print(m.eq_right(m.K(4), 4), m.eq_right(m.K(4), 5), m.eq_right("x", 4))
print(m.ne_left(m.K(4), 4), m.ne_left(m.K(4), 5))
print(m.plain_eq(m.P(1), 1))
xs = m.as_list(3)
print(type(xs).__name__, len(xs), xs == [m.K(3), 7, m.K(4)], xs.count(m.K(3)), xs.count(m.K(9)))
print(m.K(3) in xs, xs.index(m.K(4)))
ys = m.same(5)
print(ys[0] is ys[1], ys[0] == ys[1], ys[0] == ys[2], ys.count(ys[0]), len(set(ys)))
"#;

/// What CPython prints for [`DRIVER`] over [`MODULE`]; the extension must
/// print the same.
const EXPECTED: &str = "one two\nKeyError K\nmissing\nTrue False False\nTrue False False\n\
                        False True\nFalse\nlist 3 True 1 0\nTrue 2\nTrue True False 2 2\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_three_positions_match_cpython_in_an_ext_module() {
    let dir = ScratchDir::new("ext_1470_operands").expect("scratch");
    let src_dir = dir.join("src");
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&src_dir).expect("create the source directory");
    std::fs::create_dir_all(&out_dir).expect("create the output directory");
    let source = write(&src_dir, "keyed.py", MODULE);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(out_dir.join("keyed"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert_ok(&build);

    let oracle = run(DRIVER, &src_dir, &dir);
    assert_ok(&oracle);
    assert_eq!(stdout_of(&oracle), EXPECTED, "CPython's own answer");
    let compiled = run(DRIVER, &out_dir, &dir);
    assert_ok(&compiled);
    assert_eq!(stdout_of(&compiled), EXPECTED);
}

/// The embedded counterpart: keys and comparisons on both sides, through a
/// carrier copy (`copy.copy`) typed as an object and host dicts filled by
/// `__setitem__` (a call argument since #1435).
#[cfg(not(windows))]
const EMBEDDED: &str = r#"import builtins
import copy


class H:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other: "H") -> bool:
        return self.v == other.v

    def __hash__(self) -> int:
        return self.v


class P:
    def __init__(self, v: int) -> None:
        self.v = v


a = H(1)
b = H(1)
c = H(2)
o = copy.copy(b)
print(a == o, o == a, a != o, c == o, o == c)
d = builtins.dict()
d.__setitem__(b, 7)
d.__setitem__(c, 8)
print(d[a], d[H(2)], len(d))
p = P(3)
pd = builtins.dict()
pd.__setitem__(p, 0)
print(pd[p], pd.get(P(3), "none"), p == copy.copy(p))
"#;

/// Builds `body` as an embedded executable and answers its bundle's
/// interpreter, asserted to be CPython 3.14.7 (as
/// `tests/issue_1223_embedded_executable.rs` does).
#[cfg(not(windows))]
fn build_embedded(dir: &Path, body: &str) -> PathBuf {
    let output = pycc()
        .arg("build")
        .arg(write(dir, "m.py", body))
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(output.status.success(), "{}", stderr_of(&output));
    let marker = std::fs::read_to_string(dir.join("app.pycc").join("PYCC-BUNDLE"))
        .expect("an embedded build writes its marker");
    let mut lines = marker.lines();
    assert_eq!(lines.next(), Some("pycc-bundle 1"));
    assert_eq!(lines.next(), Some("python 3.14.7"), "{marker}");
    let executable = lines
        .next()
        .and_then(|line| line.strip_prefix("executable "))
        .expect("the marker names its interpreter");
    PathBuf::from(executable)
}

#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn keys_and_comparisons_match_cpython_3_14_7_in_an_embedded_executable() {
    let dir = ScratchDir::new("embed_1470_operands").expect("scratch");
    let python = build_embedded(&dir, EMBEDDED);
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
    let oracle = Command::new(python)
        .arg(dir.join("m.py"))
        .output()
        .expect("CPython runs the oracle program");
    assert_eq!(
        stdout_of(&oracle),
        "True True False False False\n7 8 2\n0 none False\n"
    );
    assert_eq!(embedded.stdout, oracle.stdout);
}

/// An embedded executable installs the same slots, so a dunder it cannot
/// install is a `C0003` naming the embedded executable -- answered before
/// the interpreter probe, so a missing interpreter is never reached.
#[test]
fn an_uninstallable_slot_is_refused_before_the_embed_probe() {
    let dir = ScratchDir::new("embed_1470_refusal").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(write(
            &dir,
            "m.py",
            "import json\n\n\nclass S:\n    @staticmethod\n    def __eq__(a: int, b: int) -> bool:\n        \
             return True\n\n\nprint(json.dumps(1))\n",
        ))
        .arg("-o")
        .arg(dir.join("app"))
        .env("PYCC_PYTHON", "/nonexistent/pycc-no-python")
        .output()
        .expect("pycc should spawn");
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(
        rendered.contains(
            "error[C0003]: an embedded executable cannot install `S.__eq__` as the host-visible \
             `__eq__` of `S` instances: it is bound as a `@staticmethod`"
        ),
        "{rendered}"
    );
    assert!(rendered.contains("(#1427, #1470)"), "{rendered}");
    assert!(!rendered.contains("PYCC_PYTHON"), "{rendered}");
    assert!(!dir.join("app").exists());
    assert!(!dir.join("app.pycc").exists());
}

/// The refusal covers every class of an embedded program, not only one
/// whose instances reach CPython: `L` never crosses, and the program built
/// before #1470, but its `__lt__` takes a `list[int]` the boundary cannot
/// carry, so it is refused now (a deliberate tightening, `docs/RUNTIME.md`).
#[test]
fn a_class_that_never_crosses_is_still_refused_in_an_embedded_executable() {
    let dir = ScratchDir::new("embed_1470_tightening").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(write(
            &dir,
            "m.py",
            "import json\n\n\nclass L:\n    def __init__(self, v: int) -> None:\n        \
             self.v = v\n\n    def __lt__(self, other: list[int]) -> bool:\n        \
             return self.v < len(other)\n\n\nprint(json.dumps(1), L(1).__lt__([1, 2]))\n",
        ))
        .arg("-o")
        .arg(dir.join("app"))
        .env("PYCC_PYTHON", "/nonexistent/pycc-no-python")
        .output()
        .expect("pycc should spawn");
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(
        rendered.contains(
            "error[C0003]: an embedded executable cannot install `L.__lt__` as the host-visible \
             `__lt__` of `L` instances: its parameter `other: list`"
        ),
        "{rendered}"
    );
    assert!(!dir.join("app").exists());
}
