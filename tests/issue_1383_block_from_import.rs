//! #1383: a foreign `from X import a, b` nested in a module-level `if` or
//! `try` block. It binds exactly as the top-level form does (#1278), but at
//! the statement's own position, so an untaken branch imports nothing, and
//! a failure -- a missing module or a missing *name* -- takes the #1293
//! bridge, so an enclosing `except ImportError` runs as in CPython.
//!
//! What stays refused is pinned here too: a nested from-import of a project
//! or `pycc_std` module keeps the block-body `C0001` (the project module is
//! never even loaded), and a function-body one keeps its own `C0001`. The
//! optional-dependency idiom `except ImportError: x = None`, once the
//! foreign-shadowing `C0001`, is admitted since #1485
//! (`tests/issue_1485_import_fallback.rs`).
//!
//! The hosted tests at the bottom are `#[ignore]`d and contribute no line
//! coverage; the Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The lines this change
//! needs covered are covered by the non-ignored tests here and by the unit
//! tests in `crates/pycc_hir/src/import/tests/block_from.rs`,
//! `src/modules/tests.rs` and `crates/pycc_codegen/src/foreign_import/tests.rs`.

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

/// The one error `pycc check` reports for `body` in `dir`, as rendered.
fn one_error(dir: &Path, body: &str, code: &str) -> String {
    let output = check_with(dir, body, &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]")), "{rendered}");
    rendered
}

const BLOCK_TEXT: &str = "an `import` inside a block body";

/// Before #1383 each of these was `C0001` "an `import` inside a block
/// body".
#[test]
fn check_accepts_a_foreign_from_import_in_a_block() {
    for (tag, body) in [
        (
            "block_from_if",
            "import sys\n\nif sys:\n    from itertools import product\n",
        ),
        (
            "block_from_try",
            "try:\n    from itertools import product\nexcept ImportError:\n    pass\n",
        ),
        (
            "block_from_multi",
            "if True:\n    from itertools import product, chain\n    print(product)\n",
        ),
        // Neither `os` nor `json` is a `pycc_std` module, so the dotted
        // forms are foreign as at top level (Part 1 of #1138).
        (
            "block_from_dotted",
            "if True:\n    from os.path import join\n    from json.decoder import JSONDecoder\n",
        ),
        // A handler that always leaves keeps the name bound after the block.
        (
            "block_from_try_raise",
            "try:\n    from itertools import product\nexcept ImportError:\n    \
             raise ValueError('itertools is required')\nprint(product)\n",
        ),
    ] {
        assert_checks_clean(tag, body);
    }
}

/// The identical from-import in both arms is exempt from the shadowing
/// rule (#1291), so the name is definitely bound after the block and in a
/// function body.
#[test]
fn check_accepts_a_read_after_an_if_else_that_imports_in_both_arms() {
    assert_checks_clean(
        "block_from_if_else",
        "import sys\n\nif sys:\n    from colorsys import hls_to_rgb\nelse:\n    \
         from colorsys import hls_to_rgb\n\nprint(hls_to_rgb)\n\n\
         def f() -> None:\n    print(hls_to_rgb)\n",
    );
}

/// The pre-#1383 refusal is kept for a project module, and the module is
/// never loaded: a helper with a syntax error reports nothing of its own.
#[test]
fn a_nested_from_import_of_a_project_module_keeps_the_block_diagnostic() {
    let dir = ScratchDir::new("block_from_project").expect("scratch");
    std::fs::write(dir.join("helper.py"), "def g(:\n").expect("write helper.py");
    let rendered = one_error(&dir, "if True:\n    from helper import g\n", "C0001");
    assert!(rendered.contains(BLOCK_TEXT), "{rendered}");
    assert!(rendered.contains("m.py:2:5"), "{rendered}");

    // A helper that imports the entry back is not a cycle: it is never
    // loaded.
    std::fs::write(
        dir.join("helper.py"),
        "from m import x\n\ndef g() -> None:\n    pass\n",
    )
    .expect("write helper.py");
    let rendered = one_error(&dir, "x = 1\nif True:\n    from helper import g\n", "C0001");
    assert!(rendered.contains(BLOCK_TEXT), "{rendered}");
    assert!(!rendered.contains("E0108"), "{rendered}");
}

#[test]
fn a_nested_from_import_of_a_pycc_std_module_keeps_the_block_diagnostic() {
    let dir = ScratchDir::new("block_from_std").expect("scratch");
    let rendered = one_error(&dir, "if True:\n    from math import sqrt\n", "C0001");
    assert!(rendered.contains(BLOCK_TEXT), "{rendered}");
}

#[test]
fn a_function_body_from_import_keeps_its_diagnostic() {
    let dir = ScratchDir::new("block_from_function").expect("scratch");
    let rendered = one_error(
        &dir,
        "def f() -> None:\n    from itertools import product\n",
        "C0001",
    );
    assert!(
        rendered.contains("an `import` inside a function or block body"),
        "{rendered}"
    );
}

/// The issue's own optional-dependency example. Refused as foreign
/// shadowing until #1485, which admits an `except ImportError` handler's
/// `N = None` as the fallback of the body's foreign import; the full
/// matrix is `tests/issue_1485_import_fallback.rs`.
#[test]
fn the_optional_dependency_idiom_is_admitted() {
    assert_checks_clean(
        "block_from_idiom",
        "try:\n    from itertools import product\nexcept ImportError:\n    product = None\n",
    );
}

/// The block path adds no annotation alias for a foreign name (as for the
/// plain nested form, #1291), so a nested-imported class is not usable as
/// an annotation.
#[test]
fn a_nested_from_imported_name_is_not_an_annotation() {
    let dir = ScratchDir::new("block_from_annotation").expect("scratch");
    let rendered = one_error(
        &dir,
        "import sys\nif sys:\n    from decimal import Decimal\nelse:\n    \
         from decimal import Decimal\n\ndef f(x: Decimal) -> None:\n    pass\n",
        "C0001",
    );
    assert!(
        rendered.contains("type annotation `Decimal` is not supported yet"),
        "{rendered}"
    );
}

/// The interop policy judges a nested from-import at its own span.
#[test]
fn a_denied_nested_from_import_is_reported_at_its_own_span() {
    let dir = ScratchDir::new("block_from_deny").expect("scratch");
    let output = check_with(
        &dir,
        "x = 1\nif x:\n    from json import dumps\n",
        &["--interop-policy", "deny"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[I0402]").count(), 1, "{rendered}");
    assert!(rendered.contains("m.py:3:5"), "{rendered}");
    assert!(
        rendered.contains("3 |     from json import dumps"),
        "{rendered}"
    );
}

/// A native build cannot embed `tkinter`; the refusal names the nested
/// statement, before any interpreter probe.
#[test]
fn a_native_build_refuses_a_nested_from_import_at_its_own_span() {
    let dir = ScratchDir::new("block_from_native").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(source(&dir, "x = 1\nif x:\n    from tkinter import Tk\n"))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert_eq!(rendered.matches("error[I0403]").count(), 1, "{rendered}");
    assert!(rendered.contains("m.py:3:5"), "{rendered}");
}

/// A nested from-import makes the source root's `pycc.toml` part of the
/// resolution exactly as a nested plain `import` does (#1291): a malformed
/// manifest reports the same error for both.
#[test]
fn a_malformed_manifest_fails_a_nested_from_import_like_a_nested_import() {
    let run = |tag: &str, body: &str| {
        let dir = ScratchDir::new(tag).expect("scratch");
        std::fs::write(dir.join("pycc.toml"), "[project\n").expect("write pycc.toml");
        let output = check_with(&dir, body, &[]);
        assert_ne!(output.status.code(), Some(0), "{body}");
        stdout_of(&output) + &stderr_of(&output)
    };
    let from = run(
        "block_from_manifest",
        "if True:\n    from itertools import product\n",
    );
    let plain = run("block_import_manifest", "if True:\n    import itertools\n");
    assert!(from.contains("pycc.toml"), "{from}");
    // The two scratch paths differ; everything after them must not.
    let tail = |text: &str| {
        text.lines()
            .next()
            .and_then(|line| line.split_once("pycc.toml"))
            .map(|(_, rest)| rest.to_string())
    };
    assert_eq!(tail(&from), tail(&plain));
    assert!(
        tail(&from).is_some_and(|rest| rest.contains("TOML parse error")),
        "{from}"
    );
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

fn oracle() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn python(dir: &Path, script: &str) -> Output {
    oracle()
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

/// Builds `body` as `module`, imports it in the host, and asserts its
/// stdout is CPython's own for the same source, and `expected`.
fn assert_hosted_like_cpython(tag: &str, module: &str, body: &str, expected: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    build_ext(&dir, module, body);
    let hosted = python(&dir, &format!("import {module}\n"));
    assert_ok(&hosted);
    let reference = oracle()
        .arg(dir.join("m.py"))
        .output()
        .expect("python3 should spawn");
    assert_ok(&reference);
    assert_eq!(stdout_of(&hosted), stdout_of(&reference));
    assert_eq!(stdout_of(&hosted), expected);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_taken_branch_binds_the_name_in_the_host() {
    assert_hosted_like_cpython(
        "block_from_taken",
        "pycc_block_from_taken_mod",
        "if True:\n    from colorsys import hls_to_rgb\n    \
         print(str(hls_to_rgb(0.0, 0.5, 0.0)))\n",
        "(0.5, 0.5, 0.5)\n",
    );
}

/// A branch that is not taken never imports, so a missing module is
/// harmless.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_untaken_branch_imports_nothing_in_the_host() {
    assert_hosted_like_cpython(
        "block_from_untaken",
        "pycc_block_from_untaken_mod",
        "if False:\n    from pycc_nosuch_1383 import x\nprint('loaded')\n",
        "loaded\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_module_under_except_import_error_runs_the_handler() {
    let run = run_hosted(
        "block_from_missing_module",
        "pycc_block_from_missing_module_mod",
        "try:\n    from pycc_nosuch_1383 import x\nexcept ImportError:\n    print('handled')\n",
        "import pycc_block_from_missing_module_mod\n",
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "handled\n");
}

/// CPython's "cannot import name" is an `ImportError` too, so it takes the
/// #1293 bridge like a missing module.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_name_under_except_import_error_runs_the_handler() {
    let run = run_hosted(
        "block_from_missing_name",
        "pycc_block_from_missing_name_mod",
        "try:\n    from itertools import pycc_nope\nexcept ImportError:\n    print('handled')\n",
        "import pycc_block_from_missing_name_mod\n",
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "handled\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_name_with_no_handler_raises_import_error_in_the_host() {
    let run = run_hosted(
        "block_from_missing_name_raise",
        "pycc_block_from_missing_name_raise_mod",
        "if True:\n    from itertools import pycc_nope\n",
        "try:\n    import pycc_block_from_missing_name_raise_mod\n\
         except ImportError as e:\n    assert type(e) is ImportError, type(e)\n    \
         assert 'cannot import name' in str(e), str(e)\n    print('raised')\n\
         else:\n    raise AssertionError('the import should have failed')\n",
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "raised\n");
}

/// `os.path.join` spells its separator per platform.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_dotted_nested_from_import_binds_the_attribute_in_the_host() {
    assert_hosted_like_cpython(
        "block_from_dotted_hosted",
        "pycc_block_from_dotted_mod",
        "if True:\n    from os.path import join\n    print(str(join('a', 'b')))\n",
        "a/b\n",
    );
}

/// A failure on a later name of one statement is bridged like any other:
/// the handler runs. (The earlier name stays bound, as in CPython, but no
/// read of it type-checks after a failure; the per-name store order is
/// pinned in `crates/pycc_codegen/src/foreign_import/tests.rs`.)
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_failure_on_a_later_name_runs_the_handler_in_the_host() {
    assert_hosted_like_cpython(
        "block_from_partial",
        "pycc_block_from_partial_mod",
        "try:\n    from itertools import product, pycc_nope\nexcept ImportError:\n    \
         print('handled')\n",
        "handled\n",
    );
}

/// A plain (embedded) build compiles its module with `ext` set, so a nested
/// from-import runs there exactly as under `--ext`, and the output matches
/// CPython's own.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_runs_a_nested_from_import_like_cpython() {
    let dir = ScratchDir::new("block_from_embedded").expect("scratch");
    let body = "if True:\n    from colorsys import hls_to_rgb\n    \
                print(str(hls_to_rgb(0.0, 0.5, 0.0)))\n\
                if False:\n    from json import dumps\nprint('done')\n";
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
    let reference = oracle()
        .arg(dir.join("m.py"))
        .output()
        .expect("python3 should spawn");
    assert_ok(&reference);
    assert_eq!(stdout_of(&embedded), stdout_of(&reference));
    assert_eq!(stdout_of(&embedded), "(0.5, 0.5, 0.5)\ndone\n");
}

/// Under `--ext --foreign-relative-imports` the entry module's relative
/// from-import may stand in a module-level `if`/`try` block too: it resolves
/// against the package the artifact is imported under, an untaken branch
/// imports nothing, and a missing sibling's `ModuleNotFoundError` is caught
/// by `except ImportError`, exactly as the same `.py` installed there.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_nested_relative_from_import_binds_the_host_package_s_object() {
    let dir = ScratchDir::new("block_from_relative").expect("scratch");
    let artifact = if cfg!(windows) { "m.pyd" } else { "m.abi3.so" };
    for (file, text) in [("top/__init__.py", ""), ("top/sib.py", "x = 41\n")] {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, text).expect("write the package");
    }
    std::fs::create_dir_all(dir.join("src")).expect("mkdir src");
    let body = "try:\n    from .sib import x\n    print(str(x))\n\
                except ImportError:\n    print('no sib')\n\
                try:\n    from .absent import q\n    print(str(q))\n\
                except ImportError:\n    print('no absent')\n\
                if False:\n    from .gone import g\nprint('done')\n";
    let src = source(&dir.join("src"), body);
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("src").join("m"))
        .args(["--ext", "--foreign-relative-imports"])
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let script = "import importlib\nimportlib.import_module('top.m')\n";
    std::fs::copy(&src, dir.join("top/m.py")).expect("install m.py");
    let reference = python(&dir, script);
    std::fs::remove_file(dir.join("top/m.py")).expect("remove m.py");
    std::fs::copy(
        dir.join("src").join(artifact),
        dir.join("top").join(artifact),
    )
    .expect("install the artifact");
    let hosted = python(&dir, script);
    assert_ok(&reference);
    assert_ok(&hosted);
    assert_eq!(stdout_of(&hosted), stdout_of(&reference));
    assert_eq!(stdout_of(&hosted), "41\nno absent\ndone\n");
}
