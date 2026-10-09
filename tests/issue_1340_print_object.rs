//! #1340: printing a CPython object value and interpolating it into an
//! f-string.
//!
//! `print(o)` writes CPython's `str(o)` and `f"{o}"` writes CPython's
//! `format(o, "")`, which calls `__format__`, not `__str__`. Conversion
//! happens where CPython does it: `print` evaluates every argument first
//! and converts each one as it writes it, while an f-string converts each
//! part as it reaches it. A raising `__str__` or `__format__` raises
//! CPython's own exception, which a function's handler catches.
//!
//! Each case builds `m.py` as an extension and runs a host script against
//! it, then runs the same script against the same source imported by
//! CPython itself, and asserts both runs print the pinned stdout. Helper
//! modules sit on a host-only `PYTHONPATH`. Both runs use
//! `PYTHONUNBUFFERED=1`: pycc writes straight to the file descriptor while
//! CPython buffers a piped stdout, so interleaved output would otherwise
//! reorder.
//!
//! Every hosted test here is `#[ignore]`d and contributes no line
//! coverage; the Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by `crates/pycc_codegen/src/string_render_tests.rs`,
//! `crates/pycc_types/src/foreign/tests.rs` and
//! `src/ext_build_tests/object_text.rs`.

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

fn oracle() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

/// The host-only helper module every ordering and failure case imports.
/// `Probe` renders the counter's value at the moment it is converted, so
/// the output shows when each conversion ran.
const HH: &str = "count = 0\n\
    class Probe:\n    def __str__(self):\n        return f'probe{count}'\n\
    class Bad:\n    def __str__(self):\n        raise ValueError('no str')\n    \
    def __format__(self, spec):\n        raise ValueError('no format')\n\
    class Fmt:\n    def __str__(self):\n        return 'S'\n    \
    def __format__(self, spec):\n        return 'F'\n\
    def bump():\n    global count\n    count += 1\n    return count\n";

/// Writes the `hh` helper module into `dir/hostlib`, the host-only
/// `PYTHONPATH` entry.
fn write_helpers(dir: &Path) {
    let lib = dir.join("hostlib");
    std::fs::create_dir_all(&lib).expect("create hostlib");
    std::fs::write(lib.join("hh.py"), HH).expect("write the helper module");
}

/// Builds `body` as the extension module `module` inside `dir`.
fn build_ext(dir: &Path, module: &str, body: &str) {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .env("PYTHONPATH", dir.join("hostlib"))
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
}

/// Runs `script` in the host with `dir` first on `sys.path`.
fn python(dir: &Path, script: &str) -> Output {
    oracle()
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("PYTHONPATH", dir.join("hostlib"))
        .env("PYTHONUNBUFFERED", "1")
        .output()
        .expect("python3 should spawn")
}

fn assert_ok(run: &Output, what: &str) {
    assert!(
        run.status.success(),
        "{what} -- stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

/// Runs `script` against `body` built by pycc as `module`, and against
/// `body` imported as `module` by CPython, and asserts both print
/// `expected`.
fn assert_matches_cpython(tag: &str, module: &str, body: &str, script: &str, expected: &str) {
    let hosted = ScratchDir::new(tag).expect("scratch");
    write_helpers(&hosted);
    build_ext(&hosted, module, body);
    let run = python(&hosted, script);
    assert_ok(&run, "pycc");
    assert_eq!(stdout_of(&run), expected, "pycc on {body}");

    let reference = ScratchDir::new(&format!("{tag}_cpython")).expect("scratch");
    write_helpers(&reference);
    std::fs::write(reference.join(format!("{module}.py")), body).expect("write the oracle source");
    let cpython = python(&reference, script);
    assert_ok(&cpython, "CPython");
    assert_eq!(stdout_of(&cpython), expected, "CPython on {body}");
}

/// A host script that imports `module` and prints what the import raised.
fn import_report(module: &str) -> String {
    format!(
        "try:\n    import {module}\nexcept Exception as e:\n    \
         print(type(e).__name__, e)\nelse:\n    print('no error')\n"
    )
}

/// The issue's own program.
const ISSUE: &str = "from itertools import product\n\n\
    x = product(\"ab\", \"c\")\nfor t in x:\n    print(t)\n";

#[test]
fn the_issue_program_checks_clean() {
    let dir = ScratchDir::new("print1340_check").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, ISSUE).expect("write the fixture source");
    let output = pycc()
        .arg("check")
        .arg(&src)
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

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_issue_program_prints_like_cpython() {
    assert_matches_cpython(
        "print1340_issue",
        "pycc_print1340_issue",
        ISSUE,
        "import pycc_print1340_issue\n",
        "('a', 'c')\n('b', 'c')\n",
    );
}

/// Objects mixed with native values in one `print`, and an object
/// interpolated between literal text.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn objects_mix_with_native_arguments_and_literal_text() {
    assert_matches_cpython(
        "print1340_mixed",
        "pycc_print1340_mixed",
        "from itertools import product\n\n\
         x = product(\"a\", \"bc\")\nfor t in x:\n    print(\"t =\", t, 2, None, t)\n    \
         print(f\"<{t}> and {t}!\")\n",
        "import pycc_print1340_mixed\n",
        "t = ('a', 'b') 2 None ('a', 'b')\n<('a', 'b')> and ('a', 'b')!\n\
         t = ('a', 'c') 2 None ('a', 'c')\n<('a', 'c')> and ('a', 'c')!\n",
    );
}

/// `print` converts after evaluating every argument, an f-string converts
/// each part as it reaches it, and `{o}` calls `__format__` where `print`
/// calls `__str__`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn conversion_order_and_protocol_match_cpython() {
    assert_matches_cpython(
        "print1340_order",
        "pycc_print1340_order",
        "import hh\n\n\
         print(hh.Probe(), hh.bump())\n\
         print(f\"{hh.Probe()} {hh.bump()} {hh.Probe()}\")\n\
         print(hh.Fmt(), f\"{hh.Fmt()}\")\n",
        "import pycc_print1340_order\n",
        "probe1 1\nprobe1 2 probe2\nS F\n",
    );
}

/// A raising `__str__` in module code fails the import with CPython's own
/// exception, after `print` has already written the earlier argument and
/// its separator.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_str_fails_the_import_like_cpython() {
    assert_matches_cpython(
        "print1340_raise_print",
        "pycc_print1340_rp",
        "import hh\n\nprint(\"a\", hh.Bad())\n",
        &import_report("pycc_print1340_rp"),
        "a ValueError no str\n",
    );
}

/// A raising `__format__` in module code fails the import before anything
/// is written.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_format_fails_the_import_like_cpython() {
    assert_matches_cpython(
        "print1340_raise_fmt",
        "pycc_print1340_rf",
        "import hh\n\nprint(f\"a{hh.Bad()}\")\n",
        &import_report("pycc_print1340_rf"),
        "ValueError no format\n",
    );
}

/// Inside a function both renderings run, and each failure is caught by
/// the function's own handler.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_function_renders_objects_and_catches_their_failures() {
    assert_matches_cpython(
        "print1340_fn",
        "pycc_print1340_fn",
        "import hh\n\n\n\
         def show() -> None:\n    print(hh.Fmt(), f\"[{hh.Fmt()}]\")\n\n\n\
         def printed() -> str:\n    try:\n        print(hh.Bad())\n    \
         except ValueError:\n        return \"caught print\"\n    return \"missed\"\n\n\n\
         def formatted() -> str:\n    try:\n        s = f\"{hh.Bad()}\"\n        return s\n    \
         except ValueError:\n        return \"caught format\"\n",
        "import pycc_print1340_fn as m\nm.show()\nprint(m.printed())\nprint(m.formatted())\n",
        "S [F]\ncaught print\ncaught format\n",
    );
}

/// A plain (embedded) build compiles its module with `ext` set, so both
/// renderings run there too, matching CPython's own output.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_prints_objects_like_cpython() {
    let dir = ScratchDir::new("print1340_embedded").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(
        &source,
        "from itertools import product\n\n\
         x = product(\"ab\", \"c\")\nfor t in x:\n    print(t, f\"<{t}>\")\n",
    )
    .expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_ok(&embedded, "embedded");
    let cpython = oracle()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert_ok(&cpython, "CPython");
    assert_eq!(stdout_of(&embedded), stdout_of(&cpython));
    assert_eq!(
        stdout_of(&embedded),
        "('a', 'c') <('a', 'c')>\n('b', 'c') <('b', 'c')>\n"
    );
}
