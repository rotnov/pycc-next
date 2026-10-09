//! #1291 (Part 1 of #1282): a foreign `import` inside a module-level `if` or
//! `try` block. The import runs where it stands, in the block's own control
//! flow, exactly as CPython runs it: a branch that is not taken imports
//! nothing, and a missing module raises `ModuleNotFoundError` from the
//! statement that names it.
//!
//! Catching that error with `except ImportError` is #1293
//! (`tests/issue_1293_import_bridge.rs`), and the
//! `ImportError`/`ModuleNotFoundError` builtins are #1292; neither is here.
//!
//! The hosted tests at the bottom are `#[ignore]`d and contribute no line
//! coverage; the Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The lines this change
//! needs covered are covered by the non-ignored tests here and by the unit
//! tests in `crates/pycc_hir/src/import/tests/block.rs`.

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

fn check_with(dir: &Path, body: &str, extra: &[&str]) -> Output {
    pycc()
        .arg("check")
        .args(extra)
        .arg(source(dir, body))
        .output()
        .expect("pycc should spawn")
}

fn assert_checks_clean(tag: &str, body: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check_with(&dir, body, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{body}\n{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&output), "", "{body}");
}

/// Before #1291 each of these was `C0001` "an `import` inside a block body".
#[test]
fn check_accepts_a_foreign_import_in_an_if_body() {
    assert_checks_clean(
        "block_import_if",
        "import sys\n\nif sys:\n    import json\n",
    );
}

#[test]
fn check_accepts_a_foreign_import_in_a_try_body() {
    assert_checks_clean(
        "block_import_try",
        "try:\n    import json\nexcept Exception:\n    pass\n",
    );
}

#[test]
fn check_accepts_an_aliased_foreign_import_in_a_block() {
    assert_checks_clean("block_import_alias", "if True:\n    import json as j\n");
}

/// Both arms bind the name, so the read after the `if` is definitely
/// assigned (#1289 owns the one-armed shape).
#[test]
fn check_accepts_a_read_after_an_if_else_that_imports_in_both_arms() {
    assert_checks_clean(
        "block_import_if_else",
        "import sys\n\nif sys:\n    import json\nelse:\n    import json\n\nprint(str(json.dumps(1)))\n",
    );
}

/// The interop policy judges a nested import at its own span, not at the
/// enclosing block's.
#[test]
fn a_denied_block_import_is_reported_at_its_own_span() {
    let dir = ScratchDir::new("block_import_deny").expect("scratch");
    let output = check_with(
        &dir,
        "x = 1\nif x:\n    import json\n",
        &["--interop-policy", "deny"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[I0402]").count(), 1, "{rendered}");
    assert!(rendered.contains("m.py:3:5"), "{rendered}");
    assert!(rendered.contains("3 |     import json"), "{rendered}");
}

/// A native build cannot embed `tkinter`; the refusal names the nested
/// statement.
#[test]
fn a_native_build_refuses_a_block_import_at_its_own_span() {
    let dir = ScratchDir::new("block_import_native").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(source(&dir, "x = 1\nif x:\n    import tkinter\n"))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(rendered.contains("error[I0403]"), "{rendered}");
    assert!(rendered.contains("`import tkinter`"), "{rendered}");
    assert!(rendered.contains("m.py:3:5"), "{rendered}");
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

/// Builds `body` as `module`, imports it in the host, and returns the run.
fn run_hosted(tag: &str, module: &str, body: &str, script: &str) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    build_ext(&dir, module, body);
    python(&dir, script)
}

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

/// The script that expects the module import to fail with
/// `ModuleNotFoundError` naming `pycc_nosuch_1291`.
fn expect_missing(module: &str) -> String {
    format!(
        "try:\n\
         \x20   import {module}\n\
         except ModuleNotFoundError as e:\n\
         \x20   assert e.name == 'pycc_nosuch_1291', e.name\n\
         else:\n\
         \x20   raise AssertionError('the import should have failed')\n"
    )
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_taken_branch_binds_the_module_in_the_host() {
    let run = run_hosted(
        "block_import_taken",
        "pycc_block_taken_mod",
        "if True:\n    import colorsys\n    print(str(colorsys.hls_to_rgb(0.0, 0.5, 0.0)))\n",
        "import pycc_block_taken_mod\n",
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "(0.5, 0.5, 0.5)\n");
}

/// A branch that is not taken never imports its module, so a missing one
/// is harmless.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_untaken_branch_imports_nothing_in_the_host() {
    let run = run_hosted(
        "block_import_untaken",
        "pycc_block_untaken_mod",
        "if False:\n    import pycc_nosuch_1291\nprint('loaded')\n",
        "import pycc_block_untaken_mod\n",
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "loaded\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_taken_branch_with_a_missing_module_raises_in_the_host() {
    let run = run_hosted(
        "block_import_missing",
        "pycc_block_missing_mod",
        "if True:\n    import pycc_nosuch_1291\n",
        &expect_missing("pycc_block_missing_mod"),
    );
    assert_ok(&run);
}

/// Inside `try`, a handler that does not match lets the failure propagate
/// out of module init after `finally` runs, and the host still sees
/// CPython's original `ModuleNotFoundError`: since #1293 the failure is
/// bridged to a pycc exception inside the module body, and the shim
/// re-raises the original object when that exception escapes.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_module_in_a_try_body_propagates_in_the_host() {
    let run = run_hosted(
        "block_import_try_missing",
        "pycc_block_try_missing_mod",
        "try:\n    import pycc_nosuch_1291\nexcept ValueError:\n    print('handled')\n\
         finally:\n    print('finally')\n",
        &expect_missing("pycc_block_try_missing_mod"),
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "finally\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_if_else_import_is_readable_after_the_block_in_the_host() {
    let run = run_hosted(
        "block_import_if_else_hosted",
        "pycc_block_if_else_mod",
        "flag = False\nif flag:\n    import colorsys\nelse:\n    import colorsys\n\
         print(str(colorsys.hls_to_rgb(0.0, 0.5, 0.0)))\n",
        "import pycc_block_if_else_mod\n",
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "(0.5, 0.5, 0.5)\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_aliased_import_binds_at_top_level_and_in_a_block_in_the_host() {
    let run = run_hosted(
        "block_import_alias_hosted",
        "pycc_block_alias_mod",
        "import colorsys as c\nprint(str(c.hls_to_rgb(0.0, 0.5, 0.0)))\n\
         if True:\n    import colorsys as d\n    print(str(d.hls_to_rgb(0.0, 0.5, 0.0)))\n",
        "import pycc_block_alias_mod\n",
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "(0.5, 0.5, 0.5)\n(0.5, 0.5, 0.5)\n");
}

/// The import runs between the statements around it, in source order.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_block_import_runs_in_source_order_in_the_host() {
    let run = run_hosted(
        "block_import_order",
        "pycc_block_order_mod",
        "print('before')\nif True:\n    print('in')\n    import pycc_nosuch_1291\n    print('after')\n",
        &expect_missing("pycc_block_order_mod"),
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "before\nin\n");
}

/// The same `import X` twice binds the same module object both times.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_duplicated_import_binds_the_same_module_in_the_host() {
    let run = run_hosted(
        "block_import_duplicate",
        "pycc_block_duplicate_mod",
        "import colorsys\nimport colorsys\nprint(str(colorsys.hls_to_rgb(0.0, 0.5, 0.0)))\n",
        "import pycc_block_duplicate_mod\n",
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "(0.5, 0.5, 0.5)\n");
}

/// A plain (embedded) build compiles its module with `ext` set, so a block
/// import runs there exactly as under `--ext`, and the output matches
/// CPython's own. Both roots are standard library, because a non-standard
/// root needs a `pycc.lock` in an embedded build (#1242).
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_runs_a_block_import_like_cpython() {
    let dir = ScratchDir::new("block_import_embedded").expect("scratch");
    let body = "if True:\n    import colorsys as c\n    print(str(c.hls_to_rgb(0.0, 0.5, 0.0)))\n\
                if False:\n    import json\nprint('done')\n";
    let build = pycc()
        .arg("build")
        .arg(source(&dir, body))
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_ok(&embedded);
    let oracle = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg(dir.join("m.py"))
        .output()
        .expect("python3 should spawn");
    assert_ok(&oracle);
    assert_eq!(stdout_of(&embedded), stdout_of(&oracle));
    assert_eq!(stdout_of(&embedded), "(0.5, 0.5, 0.5)\ndone\n");
}
