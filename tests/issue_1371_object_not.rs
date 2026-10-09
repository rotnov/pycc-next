//! Part 10 of #1371: `not o` on a CPython object, in a value position, a
//! condition, a `-> bool` return, a module body and a function body. The
//! result is a native `bool`: CPython's `not` is the negated
//! `PyObject_IsTrue` (`pycc_ext_obj_truthy`), which runs the object's own
//! `__bool__` or `__len__` and may raise.
//!
//! Every hosted test builds the source with `pycc build --ext`, imports the
//! artifact into the host CPython, and compares its output with CPython's
//! own run of the same source. The hosted tests are `#[ignore]`d and
//! contribute no line coverage; the Tier-1 `native-build-test` leg runs them
//! with `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in `crates/pycc_types/src/unop.rs`,
//! `crates/pycc_types/src/foreign/tests.rs` and
//! `crates/pycc_codegen/src/tests/object_not.rs`.
//!
//! The helper module `pycc_not_helper` lives in a `host_only/` subdirectory
//! of the scratch dir rather than beside the fixture source: a `.py` next to
//! the source is resolved as a *project* import, so it would never reach
//! the foreign-object path. Every hosted child gets `host_only` on its
//! `PYTHONPATH`, and `PYTHONIOENCODING=utf-8`.

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

fn check_with(dir: &Path, body: &str) -> Output {
    pycc()
        .arg("check")
        .arg(write(dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

/// The host-only helper. `Flag` logs every truth test of it, so the log
/// shows exactly which operands were tested; `Boom`'s truth test raises;
/// `Sized` is falsy through `__len__`; `side` logs that it was evaluated.
const HELPER: &str = "log = []\n\
    \n\
    \n\
    class Flag:\n    \
    def __init__(self, value, name):\n        self.value = value\n        self.name = name\n\n    \
    def __bool__(self):\n        log.append(self.name)\n        return self.value\n\n    \
    def __repr__(self):\n        return self.name\n\
    \n\
    \n\
    class Boom:\n    \
    def __bool__(self):\n        raise ValueError(\"no truth value\")\n\
    \n\
    \n\
    class Sized:\n    \
    def __len__(self):\n        return 0\n\
    \n\
    \n\
    def side(x):\n    log.append(\"side \" + str(x))\n    return x\n\
    \n\
    \n\
    yes = Flag(True, \"yes\")\n\
    no = Flag(False, \"no\")\n\
    empty = []\n\
    pair = [1, 2]\n\
    zero = 0\n\
    two = 2\n\
    none = None\n\
    boom = Boom()\n\
    sized0 = Sized()\n";

/// The shapes Part 10 admits: falsy objects (`[]`, `0`, `None`, a `__len__`
/// of 0, a `__bool__` returning `False`) and truthy ones, a value binding,
/// a condition, a `while` test, a `-> bool` return of a produced object,
/// and a raising `__bool__` caught in value and in truth context.
///
/// `pin` repeats `not` 200 times on *mortal* objects and compares their
/// reference counts before and after: `not` borrows its operand, so a
/// retained or released operand would move its count.
const SUCCESS: &str = "import builtins\n\
    import sys\n\
    import pycc_not_helper\n\
    \n\
    H = pycc_not_helper\n\
    print(not H.yes)\n\
    print(not H.no)\n\
    print(not H.empty, not H.pair, not H.zero, not H.two, not H.none, not H.sized0)\n\
    b = not H.two\n\
    print(b)\n\
    if not H.no:\n    print(\"if not\")\n\
    print(H.log)\n\
    \n\
    \n\
    def as_bool(n: int) -> bool:\n    return not H.side(n)\n\
    \n\
    \n\
    def body(n: int) -> None:\n    \
    print(not H.yes)\n    \
    x = not H.empty\n    \
    print(x)\n    \
    if not H.pair:\n        print(\"unreached\")\n    else:\n        print(\"else\")\n    \
    k = 0\n    \
    while not H.zero and k < 3:\n        \
    k += 1\n    \
    print(k)\n    \
    print(as_bool(n))\n    \
    try:\n        print(not H.boom)\n    except ValueError:\n        print(\"ValueError value\")\n    \
    try:\n        if not H.boom:\n            print(\"unreached\")\n    \
    except ValueError:\n        print(\"ValueError truth\")\n\
    \n\
    \n\
    def pin() -> None:\n    \
    probe = builtins.object()\n    \
    e = builtins.list()\n    \
    before_p = sys.getrefcount(probe)\n    \
    before_e = sys.getrefcount(e)\n    \
    i = 0\n    \
    while i < 200:\n        \
    if not e:\n            i += 1\n        \
    if not probe:\n            i = 1000\n    \
    print(i)\n    \
    print(sys.getrefcount(probe) == before_p)\n    \
    print(sys.getrefcount(e) == before_e)\n\
    \n\
    \n\
    body(0)\n\
    body(3)\n\
    pin()\n\
    print(H.log)\n";

/// What `SUCCESS` prints under CPython.
const SUCCESS_OUT: &str = "False\nTrue\nTrue False True False True True\nFalse\nif not\n\
    ['yes', 'no', 'no']\n\
    False\nTrue\nelse\n3\nTrue\nValueError value\nValueError truth\n\
    False\nTrue\nelse\n3\nFalse\nValueError value\nValueError truth\n\
    200\nTrue\nTrue\n\
    ['yes', 'no', 'no', 'yes', 'side 0', 'yes', 'side 3']\n";

/// `SUCCESS` passes `pycc check`: every object in it comes from a foreign
/// import.
#[test]
fn check_accepts_the_success_program() {
    let dir = ScratchDir::new("obj_not_check").expect("scratch");
    let output = check_with(&dir, SUCCESS);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// The other unary operators keep their `T0021` on an object operand: only
/// `not` is defined for every CPython object.
#[test]
fn the_other_unary_operators_stay_refused() {
    for (tag, tail, needle) in [
        (
            "obj_not_usub",
            "print(-H.two)\n",
            "unary operator USub is not defined for `object`",
        ),
        (
            "obj_not_uadd",
            "print(+H.two)\n",
            "unary operator UAdd is not defined for `object`",
        ),
        (
            "obj_not_invert",
            "print(~H.two)\n",
            "unary operator Invert is not defined for `object`",
        ),
    ] {
        let dir = ScratchDir::new(tag).expect("scratch");
        let output = check_with(
            &dir,
            &format!("import pycc_not_helper\n\nH = pycc_not_helper\n{tail}"),
        );
        assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
        let rendered = stdout_of(&output);
        assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
        assert!(rendered.contains("error[T0021]"), "{rendered}");
        assert!(rendered.contains(needle), "{rendered}");
    }
}

/// Writes the helper module into `dir/host_only`.
fn write_helper(dir: &Path) {
    let helper_dir = dir.join("host_only");
    std::fs::create_dir_all(&helper_dir).expect("create the helper directory");
    std::fs::write(helper_dir.join("pycc_not_helper.py"), HELPER).expect("write the helper");
}

fn build_ext(dir: &Path, module: &str, body: &str) {
    let build = pycc()
        .arg("build")
        .arg(write(dir, "m.py", body))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

fn python(dir: &Path, script: &str) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("PYTHONPATH", dir.join("host_only"))
        .env("PYTHONIOENCODING", "utf-8")
        .output()
        .expect("python3 should spawn")
}

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

/// Runs `target` and prints what it raised, by type only, after whatever
/// the module body printed.
fn raised_report(target: &str) -> String {
    format!(
        "import runpy\n\
         try:\n\
         \x20   {target}\n\
         except Exception as e:\n\
         \x20   print(type(e).__name__)\n\
         else:\n\
         \x20   print('no error')\n"
    )
}

/// Builds `body`, then asserts the extension's import report matches
/// CPython's own run of the same source, and returns it.
fn assert_matches_cpython(tag: &str, module: &str, body: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    write_helper(&dir);
    build_ext(&dir, module, body);
    let compiled = python(&dir, &raised_report(&format!("import {module}")));
    assert_ok(&compiled);
    let oracle = python(&dir, &raised_report("runpy.run_path('m.py')"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    stdout_of(&compiled)
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn not_on_objects_behaves_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_not_hosted", "pycc_obj_not_mod", SUCCESS);
    assert_eq!(out, format!("{SUCCESS_OUT}no error\n"));
}

/// A raising `__bool__` propagates out of the module body, in value and in
/// truth context, and nothing after it runs.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_truth_test_propagates_from_the_module_body() {
    for (tag, module, tail) in [
        (
            "obj_not_raise_value",
            "pycc_obj_not_raise_value",
            "print(not H.boom)\nprint(\"unreached\")\n",
        ),
        (
            "obj_not_raise_truth",
            "pycc_obj_not_raise_truth",
            "if not H.boom:\n    print(\"unreached\")\nprint(\"unreached\")\n",
        ),
    ] {
        let body = format!("import pycc_not_helper\n\nH = pycc_not_helper\n{tail}");
        let out = assert_matches_cpython(tag, module, &body);
        assert_eq!(out, "ValueError\n", "{tag}");
    }
}
