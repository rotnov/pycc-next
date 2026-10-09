//! #1280: a multi-name `import a, b` statement. Each alias is resolved and
//! lowered on its own, as CPython runs `import a` then `import b`, while
//! the statement keeps one diagnostic, reported at the whole statement.
//!
//! The case that prompted it is lark's `import sys, re` in
//! `lark/utils.py` (the #1207 kill-criterion workload): neither module is
//! in `pycc_std`, so both names are foreign imports.
//!
//! The hosted tests at the bottom are `#[ignore]`d and contribute no line
//! coverage; the Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The lines this change
//! needs covered are covered by the non-ignored tests here and by the unit
//! tests in `crates/pycc_hir/src/import/tests/multi.rs`.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Writes `body` to `dir/m.py` and returns the path.
fn source(dir: &Path, body: &str) -> std::path::PathBuf {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    src
}

fn check(dir: &Path, body: &str) -> Output {
    pycc()
        .arg("check")
        .arg(source(dir, body))
        .output()
        .expect("pycc should spawn")
}

/// lark's own line. Before #1280 it was `C0001` "only a single module per
/// `import` statement is supported so far".
#[test]
fn check_accepts_a_multi_name_foreign_import() {
    let dir = ScratchDir::new("multi_import_check_ok").expect("scratch");
    let output = check(&dir, "import sys, re\n");
    assert_eq!(output.status.code(), Some(0), "{}", stdout_of(&output));
    assert_eq!(stdout_of(&output), "");
}

/// The same name twice in one statement binds it foreign twice to the same
/// module, which #1291 admits exactly as it admits two separate
/// `import numpy` statements: both bindings produce the same module object.
#[test]
fn a_name_repeated_in_one_foreign_import_is_admitted() {
    let dir = ScratchDir::new("multi_import_duplicate").expect("scratch");
    let output = check(&dir, "import numpy, numpy\n");
    assert_eq!(output.status.code(), Some(0), "{}", stdout_of(&output));
    assert_eq!(stdout_of(&output), "");
}

/// A project module among the names fails the statement with the module
/// namespace gap, and the failed statement still poisons every name it
/// would have bound: the later `class A(math)` reports no cascade.
#[test]
fn a_project_module_in_a_multi_name_import_is_one_diagnostic() {
    let dir = ScratchDir::new("multi_import_project").expect("scratch");
    std::fs::write(dir.join("pkg.py"), "x = 1\n").expect("write pkg.py");
    let output = check(&dir, "import pkg, math\n\n\nclass A(math):\n    pass\n");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(
        rendered.contains("module namespace bindings (`import pkg`) are not supported yet"),
        "{rendered}"
    );
}

/// A native build refuses the alias it cannot embed, at the whole
/// statement's span, and names that alias rather than the statement's first
/// name.
#[test]
fn a_native_build_refuses_the_unembeddable_alias_at_the_statement() {
    let dir = ScratchDir::new("multi_import_native").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(source(&dir, "x = 1\n\n\nimport sys, tkinter\n"))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(rendered.contains("`import tkinter`"), "{rendered}");
    // `sys` embeds on every host (a Windows host since #1286), so it is not
    // refused.
    assert_eq!(rendered.matches("error[I0403]").count(), 1, "{rendered}");
    assert!(!rendered.contains("`import sys`"), "{rendered}");
    assert!(rendered.contains("m.py:4:1"), "{rendered}");
    assert!(rendered.contains("4 | import sys, tkinter"), "{rendered}");
}

/// The interop allowlist judges each alias's root on its own: `json` is
/// admitted, `pprint` is the one `I0402`.
#[test]
fn an_allowlist_judges_each_alias_of_one_statement() {
    let dir = ScratchDir::new("multi_import_allowlist").expect("scratch");
    std::fs::write(
        dir.join("pycc.toml"),
        "[project]\nname = \"p\"\nentry = \"main.py\"\npython = \"3.14\"\n\n\
         [interop]\npolicy = \"allowlist\"\nallow = [\"json\"]\n",
    )
    .expect("write pycc.toml");
    std::fs::write(dir.join("main.py"), "import json, pprint\n").expect("write main.py");
    let output = pycc()
        .args(["check", "main.py"])
        .current_dir(&*dir)
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[I0402]").count(), 1, "{rendered}");
    assert!(rendered.contains("`import pprint`"), "{rendered}");
    assert!(!rendered.contains("`import json`"), "{rendered}");
}

/// Builds `body` as an extension module named `module` inside `dir`.
fn build_ext(dir: &Path, module: &str, body: &str) {
    let build = pycc()
        .arg("build")
        .arg(source(dir, body))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

fn python(dir: &Path, script: &str) -> Output {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

/// lark's line end to end: both names bind real CPython modules, and each
/// is usable.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn both_names_of_a_multi_name_import_bind_in_the_host() {
    let dir = ScratchDir::new("multi_import_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_multi_import_mod",
        "import sys, re\n\nsys.stdout.write(\"imported\\n\")\nre.purge()\n\n\
         def answer() -> int:\n    return 42\n",
    );
    let run = python(
        &dir,
        "import pycc_multi_import_mod as m\nassert m.answer() == 42, m.answer()\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(stdout_of(&run), "imported\n");
}

/// The aliases run in source order and stop at the first failure, as
/// CPython's own `import a, b, c` does: the error names the first missing
/// module, never the second.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_multi_name_import_fails_at_its_first_missing_module_in_the_host() {
    let dir = ScratchDir::new("multi_import_missing_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_multi_missing_mod",
        "import json, pycc_nosuch_a, pycc_nosuch_b\n\ndef answer() -> int:\n    return 42\n",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_multi_missing_mod\n\
         except ModuleNotFoundError as e:\n\
         \x20   assert 'pycc_nosuch_a' in str(e), str(e)\n\
         else:\n\
         \x20   raise AssertionError('the import should have failed')\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
}
