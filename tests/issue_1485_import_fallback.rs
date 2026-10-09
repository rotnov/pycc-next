//! #1485: the optional-dependency fallback of a foreign import,
//!
//! ```python
//! try:
//!     from more_itertools import product   # or `import X as product`
//! except ImportError:
//!     product = None                       # or another foreign import
//! ```
//!
//! is admitted at module level. The name is whichever binding ran, typed
//! `object` everywhere; `None` is boxed (D-258's #1475 amendment), and the
//! handler runs through the #1293 bridge exactly as in CPython. Every other
//! rebinding of a foreign import keeps the shadowing `C0001`, and a native
//! fallback value keeps a tailored one.
//!
//! The hosted and embedded tests at the bottom are `#[ignore]`d and
//! contribute no line coverage; the Tier-1 `native-build-test` leg runs
//! them with `cargo test --workspace -- --include-ignored`. The lines this
//! change needs covered are covered by the non-ignored `pycc check` tests
//! here, which refuse or admit before any interpreter is consulted, and by
//! the unit tests in `crates/pycc_hir/src/import/tests/fallback.rs` and
//! `crates/pycc_types/src/foreign/fallback_tests.rs`.

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

fn assert_checks_clean(tag: &str, body: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check(&dir, body);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{body}\n{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&output), "", "{body}");
}

/// The one error `pycc check` reports for `body`, as rendered.
fn one_error(tag: &str, body: &str, code: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check(&dir, body);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]")), "{rendered}");
    rendered
}

const SHADOW: &str = "shadowing a foreign import is not supported yet";

/// Before #1485 each of these was the foreign-shadowing `C0001`.
#[test]
fn check_accepts_the_import_fallback_idiom() {
    for (tag, body) in [
        (
            "fallback_from_none",
            "try:\n    from pycc_nosuch_1485 import product\nexcept ImportError:\n    \
             product = None\nif product is None:\n    print('fallback')\n\n\
             def has() -> bool:\n    return product is not None\n",
        ),
        (
            "fallback_plain_none",
            "try:\n    import pycc_nosuch_1485 as np\nexcept ImportError:\n    np = None\n\
             print(np is None)\n",
        ),
        (
            "fallback_from_import",
            "try:\n    from pycc_nosuch_1485 import hls_to_rgb\nexcept ImportError:\n    \
             from colorsys import hls_to_rgb\nprint(str(hls_to_rgb(0.0, 0.5, 0.0)))\n",
        ),
        (
            "fallback_plain_import",
            "try:\n    import pycc_nosuch_1485 as cs\nexcept ImportError:\n    \
             import colorsys as cs\nprint(str(cs.hls_to_rgb(0.0, 0.5, 0.0)))\n",
        ),
        (
            "fallback_bare_two_names",
            "try:\n    from pycc_nosuch_1485 import a, b\nexcept:\n    a = None\n    b = None\n\
             print(a is None, b is None)\n",
        ),
    ] {
        assert_checks_clean(tag, body);
    }
}

/// A native fallback value keeps a tailored `C0001` at the `try`.
#[test]
fn a_native_fallback_value_is_refused_with_a_tailored_message() {
    let rendered = one_error(
        "fallback_native",
        "try:\n    from itertools import product\nexcept ImportError:\n    product = 0\n",
        "C0001",
    );
    assert!(
        rendered.contains(
            "`product` is rebound in an `except ImportError` handler to a value other than \
             `None`; only a `None` or a fallback `import`/`from ... import` statement is \
             supported as the fallback of a foreign import yet"
        ),
        "{rendered}"
    );
    assert!(rendered.contains("m.py:1:1"), "{rendered}");
}

/// Any other rebinding keeps the general refusal: in `else`, in a handler
/// that does not catch a failed import, after the `try`, and in a second
/// `try` importing a different object.
#[test]
fn every_other_rebinding_keeps_the_shadowing_refusal() {
    const TRY: &str = "try:\n    from itertools import product\n";
    for (tag, tail) in [
        (
            "fallback_else",
            "except ImportError:\n    product = None\nelse:\n    product = None\n",
        ),
        (
            "fallback_value_error",
            "except ValueError:\n    product = None\n",
        ),
        (
            "fallback_after",
            "except ImportError:\n    product = None\nproduct = None\n",
        ),
        (
            "fallback_two_trys",
            "except ImportError:\n    product = None\n\
             try:\n    from functools import product\nexcept ImportError:\n    product = None\n",
        ),
        // A stray import beside a formed group: only a pair of imports both
        // recorded in the group, in different arms, is exempt.
        (
            "fallback_stray_value_error",
            "except ValueError:\n    from functools import product\n\
             except ImportError:\n    product = None\n",
        ),
        (
            "fallback_stray_else",
            "except ImportError:\n    product = None\nelse:\n    from functools import product\n",
        ),
        (
            "fallback_stray_finally",
            "except ImportError:\n    product = None\nfinally:\n    from functools import product\n",
        ),
    ] {
        let rendered = one_error(tag, &format!("{TRY}{tail}"), "C0001");
        assert!(rendered.contains(SHADOW), "{rendered}");
    }
}

/// The handler's own read of the name, with no rebinding, is still unbound.
#[test]
fn a_handler_that_only_reads_the_name_is_unbound() {
    let rendered = one_error(
        "fallback_read_only",
        "try:\n    from itertools import product\nexcept ImportError:\n    print(product)\n",
        "T0021",
    );
    assert!(
        rendered.contains("name `product` is not defined"),
        "{rendered}"
    );
}

/// A handler that does not mention the name forms no fallback, so a read
/// after the `try` is the may-be-unbound `T0041` (the handler alone is
/// pinned as compiling by `tests/issue_1383_block_from_import.rs`).
#[test]
fn a_read_after_a_handler_that_does_not_rebind_is_may_be_unbound() {
    let rendered = one_error(
        "fallback_read_after",
        "try:\n    from itertools import product\nexcept ImportError:\n    pass\nprint(product)\n",
        "T0041",
    );
    assert!(
        rendered.contains("`product` may not be bound on every path"),
        "{rendered}"
    );
    // A handler read before its own rebinding is the same `T0041`.
    let rendered = one_error(
        "fallback_read_before",
        "try:\n    from itertools import product\nexcept ImportError:\n    print(product)\n    \
         product = None\n",
        "T0041",
    );
    assert!(
        rendered.contains("`product` may not be bound on every path"),
        "{rendered}"
    );
}

/// Writes `a.py` (the fallback module) and `main.py` into one project
/// directory and checks `main.py`.
fn check_project(tag: &str, main: &str) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    std::fs::write(
        dir.join("a.py"),
        "try:\n    from pycc_nosuch_1485 import product\nexcept ImportError:\n    \
         from itertools import product\n\ndef f() -> bool:\n    return product is None\n",
    )
    .expect("write a.py");
    let entry = dir.join("main.py");
    std::fs::write(&entry, main).expect("write main.py");
    pycc()
        .arg("check")
        .arg(&entry)
        .output()
        .expect("pycc should spawn")
}

/// Across modules the fallback name keeps #1080's link rules: a sibling
/// binding the name to either arm's object, or defining it, is refused,
/// because the shared slot may hold either object; a sibling that only
/// calls into the fallback module checks clean.
#[test]
fn a_sibling_module_binding_the_fallback_name_is_refused_at_link() {
    let output = check_project("fallback_link_clean", "from a import f\n\nprint(f())\n");
    assert_eq!(output.status.code(), Some(0), "{}", stdout_of(&output));

    for (tag, main, phrase) in [
        (
            "fallback_link_import",
            "from a import f\nfrom itertools import product\n\nprint(f())\n",
            "binds `product` to the CPython object `itertools.product`",
        ),
        (
            "fallback_link_definition",
            "from a import f\n\nproduct = 1\nprint(f())\n",
            "defines `product`",
        ),
    ] {
        let output = check_project(tag, main);
        assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
        let rendered = stdout_of(&output);
        assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
        assert!(rendered.contains(phrase), "{rendered}");
        assert!(
            rendered.contains("shadowing a foreign import across modules"),
            "{rendered}"
        );
    }
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
    let hosted = oracle()
        .arg("-c")
        .arg(format!("import {module}\n"))
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_ok(&hosted);
    let reference = oracle()
        .arg(dir.join("m.py"))
        .output()
        .expect("python3 should spawn");
    assert_ok(&reference);
    assert_eq!(stdout_of(&hosted), stdout_of(&reference));
    assert_eq!(stdout_of(&hosted), expected);
}

/// The idiom's usual follow-up, `if N is None:`, and a function reading
/// the name after the fallback ran.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_module_falls_back_to_none_in_the_host() {
    assert_hosted_like_cpython(
        "fallback_none_hosted",
        "pycc_fallback_none_mod",
        "try:\n    from pycc_nosuch_1485 import product\nexcept ImportError:\n    \
         product = None\nif product is None:\n    print('fallback none')\n\n\
         def has() -> bool:\n    return product is not None\n\nprint(has())\n",
        "fallback none\nFalse\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_module_falls_back_to_another_import_in_the_host() {
    assert_hosted_like_cpython(
        "fallback_import_hosted",
        "pycc_fallback_import_mod",
        "try:\n    from pycc_nosuch_1485 import hls_to_rgb\nexcept ImportError:\n    \
         from colorsys import hls_to_rgb\nprint(str(hls_to_rgb(0.0, 0.5, 0.0)))\n",
        "(0.5, 0.5, 0.5)\n",
    );
}

/// A present module never runs the handler: the name is the object.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_present_module_keeps_the_object_in_the_host() {
    assert_hosted_like_cpython(
        "fallback_present_hosted",
        "pycc_fallback_present_mod",
        "try:\n    from colorsys import hls_to_rgb\nexcept ImportError:\n    \
         hls_to_rgb = None\nprint(hls_to_rgb is None)\n\
         print(str(hls_to_rgb(0.0, 0.5, 0.0)))\n",
        "False\n(0.5, 0.5, 0.5)\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_plain_import_falls_back_to_none_in_the_host() {
    assert_hosted_like_cpython(
        "fallback_plain_hosted",
        "pycc_fallback_plain_mod",
        "try:\n    import pycc_nosuch_1485 as np\nexcept ImportError:\n    np = None\n\
         print(np is None)\n\ndef missing() -> bool:\n    return np is None\n\n\
         print(missing())\n",
        "True\nTrue\n",
    );
}

/// A plain (embedded) build boxes the fallback `None` exactly as `--ext`
/// does (D-258's #1475 amendment). `msvcrt` is a standard-library root
/// that is absent off Windows, so the import fails and no `pycc.lock` is
/// needed (#1242), as in `tests/issue_1293_import_bridge.rs`.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_runs_the_none_fallback_like_cpython() {
    let dir = ScratchDir::new("fallback_embedded").expect("scratch");
    let body = "try:\n    from msvcrt import getch\nexcept ImportError:\n    \
                getch = None\nprint(getch is None)\n";
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
    assert_eq!(stdout_of(&embedded), "True\n");
}
