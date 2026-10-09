//! #1427: a compiled class's `__eq__`, `__ne__`, `__lt__`, `__le__`,
//! `__gt__`, `__ge__` and `__hash__` are installed on its `--ext` carrier
//! type as `Py_tp_richcompare` and `Py_tp_hash`, so the host's `==`, `<`,
//! `in`, `hash()` and dict lookups run them. Before this change the carrier
//! type inherited `object`'s slots and the issue's `c == None` printed
//! `False True` where CPython prints `True False`.
//!
//! [`DRIVER`] runs against the extension and against the same source
//! imported as plain Python, so CPython is the oracle for every line: the
//! issue's case, `NotImplemented` and the reflected operand, the derived
//! `!=`, an inherited `__eq__`, ordering with and without the reflected
//! method, `__hash__ = None` from an `__eq__`-only class, a compiled hash
//! in a dict, the `-1` hash, `__hash__` alone, `__ne__` alone, and an
//! exception raised by a compiled `__eq__`.
//!
//! [`EXT_ONLY_DRIVER`] pins the deviations D-244's #1427 amendment
//! records, each beside CPython's own answer.
//!
//! The hosted tests are `#[ignore]`d for the reason every `ext` test is:
//! they ask an installed CPython with development headers to build and
//! import the artifact; the Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The refusals are answered
//! before any interpreter is probed, so those tests need no CPython.

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

fn build_ext(source: &Path, out: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(out)
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
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

/// The issue's class, lark's `ParserState.__eq__` shape (`D`, reading
/// `other.v` off an `Any` operand), and one class per slot combination.
const MODULE: &str = r#"from dataclasses import dataclass
from typing import Any


@dataclass
class Pt:
    x: int
    y: int


class C:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other: object) -> bool:
        return other is None


class D:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other: Any) -> Any:
        if not isinstance(other, D):
            return NotImplemented
        return self.v == other.v


class Derived(D):
    pass


class Ne(D):
    def __ne__(self, other: Any) -> bool:
        return False


class Lt:
    def __init__(self, v: int) -> None:
        self.v = v

    def __lt__(self, other: Any) -> Any:
        if not isinstance(other, Lt):
            return NotImplemented
        return self.v < other.v


class H:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other: Any) -> Any:
        if not isinstance(other, H):
            return NotImplemented
        return self.v == other.v

    def __hash__(self) -> int:
        return self.v


class HashOnly:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        return self.v * 10


class NeOnly:
    def __init__(self, v: int) -> None:
        self.v = v

    def __ne__(self, other: Any) -> bool:
        return False


class Boom:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other: Any) -> bool:
        raise ValueError("boom")


class Base:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other: Any) -> bool:
        return True


class Over(Base):
    def __eq__(self, other: Any) -> bool:
        return False


class Typed:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other: "Typed") -> bool:
        if not isinstance(other, Typed):
            return False
        return self.v == other.v


class TypedBare:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other: "TypedBare") -> bool:
        return self.v == other.v


class TypedNone:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other: "TypedNone") -> bool:
        if not isinstance(other, TypedNone):
            return True
        return self.v == other.v


class IntEq:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other: int) -> bool:
        return self.v == other


class _P:
    def __init__(self, v: int) -> None:
        self.v = v

    def __eq__(self, other: Any) -> Any:
        if not isinstance(other, _P):
            return NotImplemented
        return self.v == other.v


class NC:
    v: int

    def __init__(self, table: dict[str, int]) -> None:
        self.v = table["a"]

    def get(self) -> int:
        return self.v

    def __eq__(self, other: Any) -> Any:
        if not isinstance(other, NC):
            return NotImplemented
        return self.v == other.v


def give_private(cb: Any, v: int) -> Any:
    return cb(_P(v))


def make_nc() -> NC:
    return NC({"a": 1})
"#;

const PRELUDE: &str = r#"import pycc_rc_mod as rc


def show(label, f):
    try:
        print(label, repr(f()))
    except Exception as e:
        print(label, type(e).__name__)


"#;

const DRIVER: &str = r#"c = rc.C(1)
show("issue", lambda: (c == None, c != None))
d = rc.D(1)
show("d==D1", lambda: d == rc.D(1))
show("d==D2", lambda: d == rc.D(2))
show("d!=D2", lambda: d != rc.D(2))
show("d==3", lambda: (d == 3, 3 == d, d != 3))
show("derived", lambda: (d == rc.Derived(1), rc.Derived(1) == d))
show("ne", lambda: (rc.Ne(1) != rc.Ne(2), rc.Ne(1) == rc.Ne(2)))
show("in", lambda: rc.D(2) in [rc.D(1), rc.D(2)])
show("hashD", lambda: hash(d))
show("hash None", lambda: (rc.D.__hash__ is None, rc.Derived.__hash__ is None))
a, b = rc.Lt(1), rc.Lt(2)
show("lt", lambda: (a < b, b < a, b > a, a > b))
show("le", lambda: a <= b)
show("lt eq", lambda: (a == a, a == b, a != b))
show("lt hash", lambda: hash(a) == hash(a))
show("lt3", lambda: a < 3)
show("hashH", lambda: hash(rc.H(5)))
show("dict", lambda: {rc.H(5): "x"}[rc.H(5)])
show("hashH-1", lambda: hash(rc.H(-1)))
ho = rc.HashOnly(2)
show("hash only", lambda: (hash(ho), ho == ho, ho == rc.HashOnly(2), {ho: 1}[ho]))
show("hash only lt", lambda: ho < ho)
no = rc.NeOnly(1)
show("ne only", lambda: (no == no, no == rc.NeOnly(1), no != rc.NeOnly(2)))
show("boom", lambda: rc.Boom(1) == 1)
show("boom ne", lambda: rc.Boom(1) != 1)
show("typed", lambda: (rc.Typed(1) == rc.Typed(1), rc.Typed(1) == 3, rc.Typed(1) != None))
show("dataclass", lambda: (rc.Pt(1, 2) == rc.Pt(1, 2), rc.Pt(1, 2) == rc.Pt(1, 3), rc.Pt(1, 2) != rc.Pt(1, 2)))
show("dataclass other", lambda: (rc.Pt(1, 2) == None, rc.Pt(1, 2) != (1, 2), rc.Pt(1, 2) in [None, rc.Pt(1, 2)]))
show("dataclass hash", lambda: (rc.Pt.__hash__ is None, {rc.Pt(1, 2)}))
show("private", lambda: rc.give_private(lambda p: (type(p).__name__, p == 3, p != 3), 1))
show("private self", lambda: rc.give_private(lambda p: p == p, 1))
nc = rc.make_nc()
show("unpublished self", lambda: nc == nc)
"#;

const DRIVER_OUT: &str = "issue (True, False)\n\
    d==D1 True\n\
    d==D2 False\n\
    d!=D2 True\n\
    d==3 (False, False, True)\n\
    derived (True, True)\n\
    ne (False, False)\n\
    in True\n\
    hashD TypeError\n\
    hash None (True, True)\n\
    lt (True, False, True, False)\n\
    le TypeError\n\
    lt eq (True, False, True)\n\
    lt hash True\n\
    lt3 TypeError\n\
    hashH 5\n\
    dict 'x'\n\
    hashH-1 -2\n\
    hash only (20, True, False, 1)\n\
    hash only lt TypeError\n\
    ne only (True, False, False)\n\
    boom ValueError\n\
    boom ne ValueError\n\
    typed (True, False, True)\n\
    dataclass (True, False, False)\n\
    dataclass other (False, True, True)\n\
    dataclass hash TypeError\n\
    private ('_P', False, True)\n\
    private self True\n\
    unpublished self True\n";

/// Each line differs from CPython by design (D-244's #1427 amendment);
/// [`CPYTHON_EXT_ONLY_OUT`] is CPython's answer to the same lines.
///
/// - `reflected`: published types are flat, so `Over`'s type is no
///   subtype of `Base`'s and the host never tries the subclass's reflected
///   `__eq__` first.
/// - `typed other`: a comparison whose parameter is annotated with a
///   compiled class answers `NotImplemented` for an operand of any other
///   type without running the body (as a `@dataclass`'s `__eq__` does), so
///   `==` falls back to identity where CPython's body reads `other.v` and
///   raises `AttributeError`. `typed none` is the same rule on a body that
///   would not raise: CPython's body answers `True` for `None`, the guard
///   answers identity's `False` without running it.
/// - `scalar other`: a comparison whose parameter carries any other checked
///   annotation (`other: int`) raises the boundary's ingress `TypeError` for
///   an operand of another type, where CPython runs the body.
/// - `unpublished`: the hidden carrier type of `NC`, which is not published
///   because its `__init__` takes a `dict`, compares (`n == 3` runs the
///   compiled `__eq__`, which answers `NotImplemented` before reading a
///   field) while the module, unlike CPython's, has no `NC` attribute.
///   (Since #1448 a hidden carrier type also carries its field
///   descriptors, so `private self` and `unpublished self`, whose compiled
///   bodies read `other.v`, moved to [`DRIVER`].)
/// - `uninitialized`: an object `tp_init` never filled has no instance for
///   the compiled method to run on: `TypeError: H.__hash__() called on an
///   uninitialized instance`, where CPython's body raises `AttributeError`.
const EXT_ONLY_DRIVER: &str = r#"show("reflected", lambda: (rc.Base(1) == rc.Over(1), rc.Over(1) == rc.Base(1)))
show("typed other", lambda: rc.TypedBare(1) == 3)
show("typed none", lambda: rc.TypedNone(1) == None)
show("scalar other", lambda: rc.IntEq(1) == "x")
n = rc.make_nc()
show("unpublished", lambda: (type(n).__name__, hasattr(rc, "NC"), n == 3))
u = rc.H.__new__(rc.H)
show("uninitialized", lambda: (hash(u), u == rc.H(1)))
"#;

const EXT_ONLY_OUT: &str = "reflected (True, False)\n\
    typed other False\n\
    typed none False\n\
    scalar other TypeError\n\
    unpublished ('NC', False, False)\n\
    uninitialized TypeError\n";

const CPYTHON_EXT_ONLY_OUT: &str = "reflected (False, False)\n\
    typed other AttributeError\n\
    typed none True\n\
    scalar other False\n\
    unpublished ('NC', True, False)\n\
    uninitialized AttributeError\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_host_compares_and_hashes_through_the_compiled_dunders() {
    let dir = ScratchDir::new("ext_richcompare").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    let source = write(&oracle, "pycc_rc_mod.py", MODULE);
    let build = build_ext(&source, &ext.join("pycc_rc_mod"));
    assert!(build.status.success(), "{}", stderr_of(&build));
    // Each run's working directory is the scratch root, so neither side can
    // import the module from where it runs.
    let driver = format!("{PRELUDE}{DRIVER}");
    let compiled = run(&driver, &ext, &dir);
    assert_ok(&compiled);
    let cpython = run(&driver, &oracle, &dir);
    assert_ok(&cpython);
    assert_eq!(stdout_of(&compiled), stdout_of(&cpython));
    assert_eq!(stdout_of(&compiled), DRIVER_OUT);

    let divergent = format!("{PRELUDE}{EXT_ONLY_DRIVER}");
    let compiled = run(&divergent, &ext, &dir);
    assert_ok(&compiled);
    assert_eq!(stdout_of(&compiled), EXT_ONLY_OUT);
    let cpython = run(&divergent, &oracle, &dir);
    assert_ok(&cpython);
    assert_eq!(stdout_of(&cpython), CPYTHON_EXT_ONLY_OUT);
}

/// A slot dunder the artifact cannot install is refused with a `C0003`
/// before any toolchain runs, never left out: leaving it out is the
/// identity bypass #1427 removed.
#[test]
fn a_dunder_the_artifact_cannot_install_is_a_capability_gap() {
    let dir = ScratchDir::new("ext_richcompare_gap").expect("scratch");
    for (file, body, expected) in [
        (
            "m_static.py",
            "class S:\n    @staticmethod\n    def __eq__(a: int, b: int) -> bool:\n        return True\n",
            "it is bound as a `@staticmethod`",
        ),
        (
            "m_attr.py",
            "class S:\n    __eq__ = 1\n",
            "it is bound as a class attribute",
        ),
        (
            "m_generic.py",
            "from typing import Any\n\n\nclass G[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\n    def __eq__(self, other: Any) -> Any:\n        return NotImplemented\n\n\ndef make() -> int:\n    g = G[int](1)\n    return 1\n",
            "PEP 695 generic class",
        ),
        (
            "m_param.py",
            "class S:\n    def __init__(self) -> None:\n        self.v = 1\n\n    def __lt__(self, other: dict[str, int]) -> bool:\n        return True\n",
            "the host-visible `__lt__` of `S` instances: its ",
        ),
    ] {
        let source = write(&dir, file, body);
        let out = dir.join(file.trim_end_matches(".py"));
        let output = build_ext(&source, &out);
        let stderr = stderr_of(&output);
        assert_eq!(output.status.code(), Some(1), "{file}: {stderr}");
        assert!(
            stderr.contains("error[C0003]: --ext cannot install"),
            "{file}: {stderr}"
        );
        assert!(stderr.contains(expected), "{file}: {stderr}");
        assert!(stderr.contains("(#1427)"), "{file}: {stderr}");
        assert!(!out.exists(), "{file}: a refused build leaves no artifact");
    }
}
