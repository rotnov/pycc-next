//! #1386 (Part 2 of #1367): a parameter or return annotated with a foreign
//! class, a subscripted foreign generic or a type variable crosses the
//! `pycc build --ext` boundary as the CPython object itself, with **no
//! run-time class check** (D-244's #1386 amendment; `docs/RUNTIME.md`'s
//! `object` admissibility row is the canonical statement).
//!
//! Every test here is a hosted comparison against CPython importing the same
//! source, so a later run-time `isinstance` check at the thunk would flip the
//! non-conforming rows (`str`, `int`, `None`) from a pass-through into a
//! `TypeError` and fail them. The tests are `#[ignore]`d and contribute no
//! line coverage; the Tier-1 `native-build-test` leg runs them with `cargo
//! test --workspace -- --include-ignored`. The non-ignored constructor
//! admission is `src/ext_build_tests/foreign_class_boundary.rs`; the function
//! shape is `tests/issue_1397_ext_any_object.rs`'s
//! `a_foreign_class_parameter_passes_through_unchecked`.

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

fn artifact_name() -> &'static str {
    if cfg!(windows) { "m.pyd" } else { "m.abi3.so" }
}

fn run_in(dir: &Path, script: &str) -> Output {
    host_python()
        .args(["-B", "-c", script])
        .current_dir(dir)
        .env("PYTHONUNBUFFERED", "1")
        .output()
        .expect("python3 should spawn")
}

/// Builds `body` as the extension `m`, runs `script` against it and against
/// CPython importing the same source, and asserts both succeed with the same
/// stdout, which is returned.
fn assert_matches_cpython(tag: &str, body: &str, script: &str) -> String {
    let compiled_dir = ScratchDir::new(tag).expect("scratch");
    let source_dir = ScratchDir::new(&format!("{tag}_src")).expect("scratch");
    let source = write(&source_dir, "m.py", body);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(compiled_dir.join("m"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
    assert!(compiled_dir.join(artifact_name()).is_file());
    let compiled = run_in(&compiled_dir, script);
    let oracle = run_in(&source_dir, script);
    for (what, run) in [("pycc", &compiled), ("cpython", &oracle)] {
        assert!(
            run.status.success(),
            "{what}: {}{}",
            stdout_of(run),
            stderr_of(run)
        );
    }
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    stdout_of(&compiled)
}

const BOX: &str = "from fractions import Fraction\n\
\n\
class Box:\n    \
    def __init__(self, f: Fraction):\n        \
        self.f = f\n\
\n    \
    def get(self) -> Fraction:\n        \
        return self.f\n\
\n    \
    def pick(self, g: Fraction) -> Fraction:\n        \
        return g\n\
\n    \
    @staticmethod\n    \
    def s(f: Fraction) -> Fraction:\n        \
        return f\n";

/// The constructor, a method returning the foreign class, a method taking
/// it, and a static method taking it, each given a conforming `Fraction`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_class_with_foreign_class_members_is_constructed_and_called_from_the_host() {
    let stdout = assert_matches_cpython(
        "1386_class",
        BOX,
        "import m\nfrom fractions import Fraction\n\
         b = m.Box(Fraction(3, 4))\n\
         print(b.get(), b.pick(Fraction(2, 3)), m.Box.s(Fraction(5)))\n\
         print(type(b.get()).__name__, b.get() + 1)\n",
    );
    assert_eq!(stdout, "3/4 2/3 5\nFraction 7/4\n");
}

/// A `str`, an `int` and `None` at every foreign-class parameter shape pass
/// through unchecked, exactly as CPython ignores the annotation.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_non_conforming_argument_passes_through_every_shape_unchecked() {
    let stdout = assert_matches_cpython(
        "1386_non_conforming",
        BOX,
        "import m\nfrom fractions import Fraction\n\
         b = m.Box(Fraction(3, 4))\n\
         for v in ('s', 5, None):\n    \
             print(repr(m.Box(v).get()), repr(b.pick(v)), repr(m.Box.s(v)))\n",
    );
    assert_eq!(stdout, "'s' 's' 's'\n5 5 5\nNone None None\n");
}

/// The object crosses both ways as itself: a `Fraction` subclass instance
/// comes back `is` the argument, through the constructor and a method.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_foreign_subclass_instance_keeps_its_identity() {
    let stdout = assert_matches_cpython(
        "1386_identity",
        BOX,
        "import m\nfrom fractions import Fraction\n\
         class Half(Fraction):\n    pass\n\
         f = Half(1, 2)\nb = m.Box(f)\n\
         print(b.get() is f, b.pick(f) is f, m.Box.s(f) is f, type(b.get()).__name__)\n",
    );
    assert_eq!(stdout, "True True True Half\n");
}

/// A module-level `T = TypeVar("T")` (#1394) resolves to the object, so a
/// `def ident(x: T) -> T` passes any value through; an `isinstance` check
/// against the type variable would be a `TypeError` in CPython itself.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_type_variable_parameter_passes_any_value_through() {
    let stdout = assert_matches_cpython(
        "1386_typevar",
        "from typing import TypeVar\n\nT = TypeVar(\"T\")\n\n\
         def ident(x: T) -> T:\n    return x\n",
        "import m\nxs = [1]\n\
         print(m.ident(3), m.ident('s'), m.ident(None), m.ident(xs) is xs)\n",
    );
    assert_eq!(stdout, "3 s None True\n");
}

/// A subscripted foreign generic is erased to the object, so `Queue[int]`
/// accepts a `list` the body measures with `len`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_subscripted_foreign_generic_parameter_is_not_checked() {
    let stdout = assert_matches_cpython(
        "1386_generic",
        "from queue import Queue\n\n\
         def size(q: Queue[int]) -> int:\n    return len(q)\n",
        "import m\nprint(m.size([1, 2, 3]), m.size('ab'))\n",
    );
    assert_eq!(stdout, "3 2\n");
}

/// What the narrowing leaves in place: a wrong argument count is still the
/// boundary's `TypeError`, at a constructor and at a method.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn arity_is_still_checked_at_a_foreign_class_parameter() {
    let stdout = assert_matches_cpython(
        "1386_arity",
        BOX,
        "import m\nfrom fractions import Fraction\n\
         b = m.Box(Fraction(1))\n\
         for call in (lambda: m.Box(), lambda: b.pick(), lambda: m.Box.s(1, 2)):\n    \
             try:\n        call()\n    \
             except TypeError:\n        print('TypeError')\n",
    );
    assert_eq!(stdout, "TypeError\nTypeError\nTypeError\n");
}
