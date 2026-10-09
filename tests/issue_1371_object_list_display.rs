//! Part 2d of #1371: a list display bound to a CPython object slot -- an
//! `object`/`Any`/bare-`list` annotated assignment, or an empty `[]`
//! assigned to a name whose other binding is a CPython object (lark
//! `lalr_parser_state.py` lines 99 and 101) -- builds a fresh CPython
//! `list` with every element boxed.
//!
//! Every hosted test builds the source with `pycc build --ext`, imports the
//! artifact into the host CPython, and compares its output with CPython's
//! own run of the same source where the two agree. The hosted tests are
//! `#[ignore]`d and contribute no line coverage; the Tier-1
//! `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in
//! `crates/pycc_types/src/foreign/list_display/tests.rs`,
//! `crates/pycc_hir/src/tests/comprehension_expr.rs`,
//! `crates/pycc_mir/src/obj_compare/tests.rs` and
//! `crates/pycc_codegen/src/tests/object_list_display.rs`.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    let mut command =
        Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()));
    command.env("PYTHONIOENCODING", "utf-8");
    command
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

fn build_ext_output(dir: &Path, module: &str, body: &str) -> Output {
    pycc()
        .arg("build")
        .arg(write(dir, "m.py", body))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

fn build_ext(dir: &Path, module: &str, body: &str) {
    let build = build_ext_output(dir, module, body);
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
}

fn python(dir: &Path, script: &str) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .current_dir(dir)
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
    build_ext(&dir, module, body);
    let compiled = python(&dir, &raised_report(&format!("import {module}")));
    assert_ok(&compiled);
    let oracle = python(&dir, &raised_report("runpy.run_path('m.py')"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    stdout_of(&compiled)
}

/// Every shape Part 2d admits, each in a function of its own: the three
/// annotations that lower to an object in an `ext` module, a display with
/// mixed element types, lark lines 94-101 (`s` an object slice on one
/// branch and `[]` on the other, then `callbacks[rule](s) if callbacks else
/// s`, with a truthy and an empty `callbacks`), the empty
/// display *before* the object binding, a display in a nested block, and
/// an `.append` on an empty display that became the object (the method
/// call is the #1095 foreign dispatch, so it grows the CPython list). Two
/// calls of the same function return two distinct lists.
const SUCCESS: &str = "import builtins\n\
    from typing import Any\n\
    \n\
    \n\
    def annotated() -> object:\n    \
    x: object = []\n    return x\n\
    \n\
    \n\
    def mixed(n: int) -> object:\n    \
    x: object = [n, \"a\", 2.5, True, builtins.len, n + 1]\n    return x\n\
    \n\
    \n\
    def any_list(n: int) -> object:\n    \
    x: Any = [n, n * 2]\n    return x\n\
    \n\
    \n\
    def bare(n: int) -> object:\n    \
    x: list = [n]\n    return x\n\
    \n\
    \n\
    def lark(value_stack: object, size: int, callbacks: object, rule: str) -> object:\n    \
    if size:\n        s = value_stack[-size:]\n    else:\n        s = []\n    \
    value = callbacks[rule](s) if callbacks else s\n    return value\n\
    \n\
    \n\
    def later(n: int) -> object:\n    \
    s = []\n    s = builtins.list(\"abc\")[n:]\n    return s\n\
    \n\
    \n\
    def nested(n: int) -> int:\n    \
    if n:\n        x: object = [n, n]\n        return len(x)\n    return 0\n\
    \n\
    \n\
    def appended(n: int) -> object:\n    \
    xs = []\n    xs.append(n)\n    \
    if n > 100:\n        xs = builtins.list(\"ab\")\n    return xs\n\
    \n\
    \n\
    print(annotated())\n\
    print(builtins.type(annotated()).__name__)\n\
    print(annotated() is annotated())\n\
    print(mixed(7))\n\
    print(any_list(3))\n\
    print(bare(4))\n\
    print(lark(builtins.list(\"abcdef\"), 2, builtins.__dict__, \"len\"))\n\
    print(lark(builtins.list(\"abcdef\"), 0, builtins.__dict__, \"repr\"))\n\
    print(lark(builtins.list(\"abcdef\"), 0, builtins.dict(), \"len\"))\n\
    print(later(1))\n\
    print(nested(5))\n\
    print(nested(0))\n\
    print(appended(5))\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn object_list_displays_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_list_hosted", "pycc_obj_list_mod", SUCCESS);
    assert_eq!(
        out,
        "[]\nlist\nFalse\n[7, 'a', 2.5, True, <built-in function len>, 8]\n[3, 6]\n[4]\n\
         2\n[]\n[]\n['b', 'c']\n2\n0\n[5]\nno error\n"
    );
}

/// An element that raises while the display is evaluated propagates like
/// CPython's own, and the list is never built.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_element_propagates_from_the_display() {
    let body = "import builtins\n\
        \n\
        \n\
        def zdiv(z: int) -> object:\n    \
        x: object = [builtins.len, 1 // z]\n    return x\n\
        \n\
        \n\
        print(zdiv(1))\n\
        print(zdiv(0))\n\
        print(\"unreached\")\n";
    let out = assert_matches_cpython("obj_list_raise", "pycc_obj_list_raise", body);
    assert_eq!(out, "[<built-in function len>, 1]\nZeroDivisionError\n");
}

/// The reference-count contract of `pycc_ext_obj_build_list`, measured on a
/// *mortal* `object()` held by the module across 100 calls of each
/// function:
///
/// - `keep` binds `[probe, probe]` and drops it: a bound list is still
///   leaked under #1092 (Part 1 releases only unbound temporaries), so each
///   call adds exactly the two references
///   the list holds -- one more would be a packed element leaked twice, one
///   fewer a stolen reference released.
/// - `fail` packs `probe`, then an `int` outside the packer's range, then
///   `probe` again: the packer raises `OverflowError` (the documented
///   divergence, `docs/RUNTIME.md`), and the helper releases both packed
///   `probe` references, so the count is unchanged.
/// - `zdiv` raises `ZeroDivisionError` before any element is packed, so
///   the count is unchanged.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn every_packed_element_is_consumed_exactly_once() {
    let body = "import builtins\n\
        import sys\n\
        \n\
        probe = builtins.object()\n\
        \n\
        \n\
        def rc() -> int:\n    return int(sys.getrefcount(probe))\n\
        \n\
        \n\
        def keep() -> int:\n    x: object = [probe, probe]\n    return 0\n\
        \n\
        \n\
        def fail(n: int) -> int:\n    \
        x: object = [probe, 4611686018427387903 + n, probe]\n    return 0\n\
        \n\
        \n\
        def zdiv(z: int) -> int:\n    x: object = [probe, 1 // z]\n    return 0\n";
    let dir = ScratchDir::new("obj_list_refcount").expect("scratch");
    build_ext(&dir, "pycc_obj_list_rc", body);
    let driver = "import pycc_obj_list_rc as m\n\
        for name, arg in (('keep', None), ('fail', 1), ('zdiv', 0)):\n    \
        fn = getattr(m, name)\n    \
        before = m.rc()\n    \
        raised = 'ok'\n    \
        for _ in range(100):\n        \
        try:\n            fn() if arg is None else fn(arg)\n        \
        except Exception as e:\n            raised = type(e).__name__\n    \
        print(name, raised, m.rc() - before)\n";
    let run = python(&dir, driver);
    assert_ok(&run);
    assert_eq!(
        stdout_of(&run),
        "keep ok 200\nfail OverflowError 0\nzdiv ZeroDivisionError 0\n"
    );
}

/// The shapes Part 2d does not admit keep a diagnostic, never a panic: an
/// element with no boxing helper, a dict display bound to an object, an
/// empty display with no object evidence, a non-empty display whose only
/// object evidence is another binding of the name.
#[test]
fn the_shapes_outside_part_2d_are_refused() {
    const HEAD: &str = "import builtins\n\n\n";
    for (tag, tail, code, needle) in [
        (
            "obj_list_nested",
            "def f() -> object:\n    x: object = [[1]]\n    return x\n",
            "I0404",
            "a `list[int]` element in a list display bound to a CPython object",
        ),
        (
            "obj_list_none",
            "def f() -> object:\n    x: object = [1, None]\n    return x\n",
            "I0404",
            "a `None` element in a list display bound to a CPython object",
        ),
        (
            "obj_list_dict",
            "def f() -> object:\n    x: object = {}\n    return x\n",
            "T0003",
            "an empty dict literal",
        ),
        (
            "obj_list_no_evidence",
            "def f() -> object:\n    s = []\n    return s\n",
            "T0003",
            "an empty list literal",
        ),
        (
            "obj_list_non_empty_binding",
            "def f() -> int:\n    s = builtins.list(\"ab\")[1:]\n    s = [1]\n    \
             return len(s)\n",
            "T0023",
            "cannot assign",
        ),
    ] {
        let dir = ScratchDir::new(tag).expect("scratch");
        let build = build_ext_output(&dir, "pycc_obj_list_refused", &format!("{HEAD}{tail}"));
        assert!(!build.status.success(), "{tag}");
        let rendered = format!("{}{}", stdout_of(&build), stderr_of(&build));
        assert_eq!(rendered.matches("error[").count(), 1, "{tag}: {rendered}");
        assert!(
            rendered.contains(&format!("error[{code}]")),
            "{tag}: {rendered}"
        );
        assert!(rendered.contains(needle), "{tag}: {rendered}");
    }
}

/// The binding source needs no `object` annotation, so lark's shape passes
/// `pycc check`, a native-mode check.
#[test]
fn check_accepts_the_binding_source_in_a_native_module() {
    let dir = ScratchDir::new("obj_list_check").expect("scratch");
    let body = "import builtins\n\
        \n\
        \n\
        def f(size: int) -> int:\n    \
        s = builtins.list(\"abc\")[1:]\n    \
        if size:\n        s = []\n    \
        return len(s)\n\
        \n\
        \n\
        print(f(0))\n";
    let output = pycc()
        .arg("check")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}
