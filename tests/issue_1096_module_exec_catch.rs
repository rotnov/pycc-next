//! Part 1 of #1096: a failed foreign operation in the module body of an
//! `ext` module is catchable by an enclosing module-level `try`.
//!
//! Inside a module-level `try` body, handler, `else` or `finally`, the
//! failure is bridged to a pycc exception and branches to the innermost
//! exception target, so an `except`/`finally` runs as it does under
//! CPython. With no enclosing `try` the import still fails directly, with
//! CPython's exception untouched. When a bridged exception escapes the
//! module body, the host receives CPython's *original* object.
//!
//! Each case builds `m.py` as an extension and imports it from a host
//! script, then runs the same script against the same source imported by
//! CPython itself, and asserts both runs print the same stdout. An
//! escaping exception is reported by class, module, `repr`, `.name` and
//! whether a traceback exists -- never the traceback's frames, since the
//! compiled module has no `<module>` frame of its own (pre-existing). Both
//! runs use `PYTHONUNBUFFERED=1`: pycc writes straight to the file
//! descriptor while CPython buffers a piped stdout, so interleaved output
//! would otherwise reorder.
//!
//! Every test here is `#[ignore]`d and contributes no line coverage; the
//! Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The Rust lines this
//! change adds are covered by the unit tests in
//! `crates/pycc_codegen/src/foreign_fail_tests.rs` and
//! `crates/pycc_codegen/src/foreign_import/tests.rs`.

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

/// Writes each `(name, text)` helper module into `dir/hostlib`, the
/// host-only `PYTHONPATH` entry.
fn write_helpers(dir: &Path, helpers: &[(&str, &str)]) {
    let lib = dir.join("hostlib");
    std::fs::create_dir_all(&lib).expect("create hostlib");
    for (name, text) in helpers {
        std::fs::write(lib.join(format!("{name}.py")), text).expect("write a helper module");
    }
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

/// The host script: import `module`, and report an escaping exception by
/// everything except its traceback's frames.
fn import_report(module: &str) -> String {
    format!(
        "try:\n\
         \x20   import {module}\n\
         except BaseException as e:\n\
         \x20   print('escaped', type(e).__module__, type(e).__qualname__, repr(e),\n\
         \x20         getattr(e, 'name', None), e.__traceback__ is not None)\n\
         else:\n\
         \x20   print('imported')\n"
    )
}

/// Builds `body` as `module` with `helpers` on the build's `PYTHONPATH` and
/// `late_helpers` written only after the build (a helper that raises at
/// import time), imports it in the host, then imports the same source
/// under CPython with the same helpers, and asserts both print `expected`.
fn assert_matches_cpython(
    tag: &str,
    module: &str,
    body: &str,
    helpers: &[(&str, &str)],
    late_helpers: &[(&str, &str)],
    expected: &str,
) {
    let script = import_report(module);
    let hosted = ScratchDir::new(tag).expect("scratch");
    write_helpers(&hosted, helpers);
    build_ext(&hosted, module, body);
    write_helpers(&hosted, late_helpers);
    let run = python(&hosted, &script);
    assert_ok(&run, "pycc");
    assert_eq!(stdout_of(&run), expected, "pycc on {body}");

    let reference = ScratchDir::new(&format!("{tag}_cpython")).expect("scratch");
    write_helpers(&reference, helpers);
    write_helpers(&reference, late_helpers);
    std::fs::write(reference.join(format!("{module}.py")), body).expect("write the oracle source");
    let cpython = python(&reference, &script);
    assert_ok(&cpython, "CPython");
    assert_eq!(stdout_of(&cpython), expected, "CPython on {body}");
}

/// The issue's own shape: a `TypeError` from a foreign call is caught, the
/// `finally` runs, and the statements after the `try` run too.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_foreign_failure_is_caught_and_finally_runs() {
    assert_matches_cpython(
        "mx1096_caught",
        "pycc_mx1096_caught",
        "import gc\n\
         try:\n\
         \x20   gc.set_threshold(\"x\")\n\
         \x20   print(\"not reached\")\n\
         except TypeError:\n\
         \x20   print(\"caught\")\n\
         finally:\n\
         \x20   print(\"finally ran\")\n\
         n = 1\n\
         print(n)\n",
        &[],
        &[],
        "caught\nfinally ran\n1\nimported\n",
    );
}

/// A non-matching `except` lets the failure escape after `finally`, as
/// CPython's original `AttributeError` with its `.name`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_unmatched_failure_runs_finally_and_escapes_as_the_original() {
    assert_matches_cpython(
        "mx1096_unmatched",
        "pycc_mx1096_unmatched",
        "import copy\n\
         try:\n\
         \x20   copy.nope\n\
         except TypeError:\n\
         \x20   print(\"wrong\")\n\
         finally:\n\
         \x20   print(\"finally ran\")\n\
         print(\"after\")\n",
        &[],
        &[],
        "finally ran\n\
         escaped builtins AttributeError AttributeError(\"module 'copy' has no attribute \
         'nope'\") nope True\n",
    );
}

/// An exception class only the host knows escapes through `finally` as
/// the original object, of the helper module's own class.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_host_only_class_escapes_as_the_original_object() {
    assert_matches_cpython(
        "mx1096_host_class",
        "pycc_mx1096_host_class",
        "import hh\n\
         try:\n\
         \x20   hh.boom()\n\
         finally:\n\
         \x20   print(\"finally ran\")\n",
        &[(
            "hh",
            "class HostOnly(Exception):\n    pass\n\n\ndef boom():\n    raise HostOnly('x')\n",
        )],
        &[],
        "finally ran\nescaped hh HostOnly HostOnly('x') None True\n",
    );
}

/// `KeyboardInterrupt` from `__len__` is a `BaseException`: `except
/// Exception` does not catch it, a bare `except` does, and with no bare
/// `except` it escapes as the original.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_base_exception_skips_except_exception() {
    let hh = "class K:\n    def __len__(self):\n        raise KeyboardInterrupt\n\n\nk = K()\n";
    assert_matches_cpython(
        "mx1096_base_caught",
        "pycc_mx1096_base_caught",
        "import hh\n\
         try:\n\
         \x20   try:\n\
         \x20       len(hh.k)\n\
         \x20   except Exception:\n\
         \x20       print(\"wrong\")\n\
         except:\n\
         \x20   print(\"bare caught\")\n\
         print(\"after\")\n",
        &[("hh", hh)],
        &[],
        "bare caught\nafter\nimported\n",
    );
    assert_matches_cpython(
        "mx1096_base_escapes",
        "pycc_mx1096_base_escapes",
        "import hh\n\
         try:\n\
         \x20   len(hh.k)\n\
         except Exception:\n\
         \x20   print(\"wrong\")\n\
         print(\"after\")\n",
        &[("hh", hh)],
        &[],
        "escaped builtins KeyboardInterrupt KeyboardInterrupt() None True\n",
    );
}

/// A foreign `for` whose `__next__` raises after one item is caught.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_foreign_iterator_is_caught() {
    assert_matches_cpython(
        "mx1096_iter",
        "pycc_mx1096_iter",
        "import hh\n\
         try:\n\
         \x20   for x in hh.gen():\n\
         \x20       print(len(x))\n\
         except ValueError:\n\
         \x20   print(\"caught\")\n\
         print(\"after\")\n",
        &[(
            "hh",
            "def gen():\n    yield 'abc'\n    raise ValueError('mid')\n",
        )],
        &[],
        "3\ncaught\nafter\nimported\n",
    );
}

/// A nested import whose module raises `ValueError` -- not an
/// `ImportError` -- is caught by `except ValueError`, and escapes `except
/// TypeError` as the original object.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_nested_import_raising_a_non_import_error_is_catchable() {
    let raises = [("pycc_raises_1096", "raise ValueError('boom')\n")];
    assert_matches_cpython(
        "mx1096_import_caught",
        "pycc_mx1096_import_caught",
        "try:\n\
         \x20   import pycc_raises_1096\n\
         except ValueError:\n\
         \x20   print(\"caught\")\n\
         print(\"after\")\n",
        &[],
        &raises,
        "caught\nafter\nimported\n",
    );
    assert_matches_cpython(
        "mx1096_import_escapes",
        "pycc_mx1096_import_escapes",
        "try:\n\
         \x20   import pycc_raises_1096\n\
         except TypeError:\n\
         \x20   print(\"wrong\")\n\
         print(\"after\")\n",
        &[],
        &raises,
        "escaped builtins ValueError ValueError('boom') None True\n",
    );
}

/// The #1485 optional-dependency fallback agrees with this routing: an
/// import that raises `ValueError` skips the `except ImportError` fallback
/// and escapes the module as the original object. (A fallback `try` nested
/// in another `try` is not an admitted #1485 shape, so the caught variant
/// is `a_nested_import_raising_a_non_import_error_is_catchable`'s.)
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_non_import_error_skips_the_import_error_fallback() {
    let raises = [("pycc_raises_1096", "raise ValueError('boom')\n")];
    assert_matches_cpython(
        "mx1096_fallback_escapes",
        "pycc_mx1096_fallback_escapes",
        "try:\n\
         \x20   import pycc_raises_1096 as m\n\
         except ImportError:\n\
         \x20   m = None\n\
         \x20   print(\"wrong fallback\")\n\
         print(m is None)\n",
        &[],
        &raises,
        "escaped builtins ValueError ValueError('boom') None True\n",
    );
}

/// A failure inside an `except` handler is caught by a `try` nested in
/// that handler.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_failure_inside_a_handler_is_caught_by_an_inner_try() {
    assert_matches_cpython(
        "mx1096_in_handler",
        "pycc_mx1096_in_handler",
        "import copy\n\
         try:\n\
         \x20   raise ValueError(\"v\")\n\
         except ValueError:\n\
         \x20   try:\n\
         \x20       copy.nope\n\
         \x20   except AttributeError:\n\
         \x20       print(\"inner caught\")\n\
         print(\"after\")\n",
        &[],
        &[],
        "inner caught\nafter\nimported\n",
    );
}

/// A failure in a `try`'s `else` body skips that `try`'s own matching
/// `except`, runs its `finally`, and escapes.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_else_body_failure_skips_its_own_handler() {
    assert_matches_cpython(
        "mx1096_else",
        "pycc_mx1096_else",
        "import copy\n\
         try:\n\
         \x20   print(\"body\")\n\
         except AttributeError:\n\
         \x20   print(\"wrong\")\n\
         else:\n\
         \x20   copy.nope\n\
         finally:\n\
         \x20   print(\"finally ran\")\n\
         print(\"after\")\n",
        &[],
        &[],
        "body\nfinally ran\n\
         escaped builtins AttributeError AttributeError(\"module 'copy' has no attribute \
         'nope'\") nope True\n",
    );
}

/// A failure in a `finally` body escapes on its own, and an outer `try`
/// catches it.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_finally_body_failure_escapes_or_is_caught_outside() {
    assert_matches_cpython(
        "mx1096_finally_escapes",
        "pycc_mx1096_finally_escapes",
        "import copy\n\
         try:\n\
         \x20   print(\"body\")\n\
         finally:\n\
         \x20   copy.nope\n\
         print(\"after\")\n",
        &[],
        &[],
        "body\n\
         escaped builtins AttributeError AttributeError(\"module 'copy' has no attribute \
         'nope'\") nope True\n",
    );
    assert_matches_cpython(
        "mx1096_finally_caught",
        "pycc_mx1096_finally_caught",
        "import copy\n\
         try:\n\
         \x20   try:\n\
         \x20       print(\"body\")\n\
         \x20   finally:\n\
         \x20       copy.nope\n\
         except AttributeError:\n\
         \x20   print(\"outer caught\")\n\
         print(\"after\")\n",
        &[],
        &[],
        "body\nouter caught\nafter\nimported\n",
    );
}

/// A bare `raise` in the handler re-raises CPython's original object.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_bare_raise_restores_the_original() {
    assert_matches_cpython(
        "mx1096_reraise",
        "pycc_mx1096_reraise",
        "import copy\n\
         try:\n\
         \x20   copy.nope\n\
         except AttributeError:\n\
         \x20   print(\"handler\")\n\
         \x20   raise\n",
        &[],
        &[],
        "handler\n\
         escaped builtins AttributeError AttributeError(\"module 'copy' has no attribute \
         'nope'\") nope True\n",
    );
}

/// A `try` inside a module-level loop catches the same failure on every
/// trip.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_loop_catches_on_every_trip() {
    assert_matches_cpython(
        "mx1096_loop",
        "pycc_mx1096_loop",
        "import copy\n\
         n = 0\n\
         for i in range(3):\n\
         \x20   try:\n\
         \x20       copy.nope\n\
         \x20   except AttributeError:\n\
         \x20       n += 1\n\
         print(n)\n",
        &[],
        &[],
        "3\nimported\n",
    );
}

/// With no enclosing `try` the edge is unchanged: the body stops and the
/// import fails with CPython's exception.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_top_level_failure_is_unchanged() {
    assert_matches_cpython(
        "mx1096_top_level",
        "pycc_mx1096_top_level",
        "import copy\n\
         print(\"a\")\n\
         copy.nope\n\
         print(\"b\")\n",
        &[],
        &[],
        "a\n\
         escaped builtins AttributeError AttributeError(\"module 'copy' has no attribute \
         'nope'\") nope True\n",
    );
}

/// An embedded build (D-248) runs the same `pycc_ext_exec_module` over the
/// same `ext` codegen, so the caught failure runs its handler and `finally`
/// exactly as CPython does. The gate and interpreter selection are
/// `tests/issue_1293_import_bridge.rs`'s `run_embedded`, unchanged.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_catches_a_module_body_foreign_failure() {
    let dir = ScratchDir::new("mx1096_embedded").expect("scratch");
    let body = "import gc\n\
                try:\n\
                \x20   gc.set_threshold(\"x\")\n\
                except TypeError:\n\
                \x20   print(\"caught\")\n\
                finally:\n\
                \x20   print(\"fin\")\n";
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    let cpython = oracle().arg(&src).output().expect("python3 should spawn");
    assert_ok(&embedded, "embedded");
    assert_ok(&cpython, "CPython");
    assert_eq!(stdout_of(&embedded), "caught\nfin\n");
    assert_eq!(stdout_of(&embedded), stdout_of(&cpython));
}
