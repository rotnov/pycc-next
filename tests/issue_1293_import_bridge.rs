//! #1293 (Part 3 of #1282): a foreign import nested in a module-level `if`
//! or `try` block whose import fails with an `ImportError` raises a pycc
//! exception, so an enclosing `except ImportError`/`except
//! ModuleNotFoundError`/`except Exception`/bare `except`/`except*`/`finally`
//! runs as it does under CPython.
//!
//! When the bridged exception escapes the module body unchanged through a
//! plain `try` (unmatched, re-raised with a bare `raise`, or after
//! `finally`), the shim re-raises CPython's *original* object, so the host
//! still sees its `.name`. One recorded deviation is pinned here as such:
//! an escape through `except*` reaches the host as a rebuilt `Exception`.
//! An import that fails with anything other than an `ImportError` is
//! bridged by the object bridge since Part 1 of #1096, so a matching
//! `except` catches it too.
//!
//! Every test here is `#[ignore]`d and contributes no line coverage; the
//! Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The Rust lines this change
//! adds are covered by `crates/pycc_codegen/src/foreign_import.rs`'s unit
//! tests and by `src/ext_build_tests/toolchain.rs`'s drift guard. Each
//! expected stdout was checked against CPython 3.14 running the same source
//! with the module absent.

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

/// Writes `body` to `dir/m.py` and returns the path.
fn source(dir: &Path, body: &str) -> std::path::PathBuf {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    src
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

/// Runs `script` in the host with `dir` as its working directory, which
/// `python -c` puts first on `sys.path`.
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

/// Imports `module`, which must load cleanly, then runs `body` in CPython
/// with every `pycc_nosuch_*` name absent and returns both stdouts.
fn assert_matches_cpython(tag: &str, module: &str, body: &str, expected: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    build_ext(&dir, module, body);
    let run = python(&dir, &format!("import {module}\n"));
    assert_ok(&run);
    assert_eq!(stdout_of(&run), expected, "{body}");
    let cpython = oracle()
        .arg(dir.join("m.py"))
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_ok(&cpython);
    assert_eq!(stdout_of(&cpython), expected, "CPython disagrees on {body}");
}

/// A host script that expects importing `module` to fail with the original
/// `ModuleNotFoundError` naming `missing`.
fn expect_original(module: &str, missing: &str) -> String {
    format!(
        "try:\n\
         \x20   import {module}\n\
         except ModuleNotFoundError as e:\n\
         \x20   assert type(e) is ModuleNotFoundError, type(e)\n\
         \x20   assert e.name == '{missing}', e.name\n\
         else:\n\
         \x20   raise AssertionError('the import should have failed')\n"
    )
}

/// The subclass path: `ModuleNotFoundError` is caught by `except
/// ImportError`, and the caught message renders like CPython's three times
/// over (#1298's borrowed-message case).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn subclass_caught_by_except_import_error() {
    assert_matches_cpython(
        "bridge_import_error",
        "pycc_bridge_ie_mod",
        "try:\n    import pycc_nosuch_1293\nexcept ImportError as e:\n    print(e)\n\
         \x20   print(f\"{e}\")\n    print(e)\nprint('after')\n",
        "No module named 'pycc_nosuch_1293'\n\
         No module named 'pycc_nosuch_1293'\n\
         No module named 'pycc_nosuch_1293'\n\
         after\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn caught_by_except_module_not_found_error() {
    assert_matches_cpython(
        "bridge_mnfe",
        "pycc_bridge_mnfe_mod",
        "try:\n    import pycc_nosuch_1293\nexcept ModuleNotFoundError as e:\n    print('caught', e)\n",
        "caught No module named 'pycc_nosuch_1293'\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn caught_by_except_exception_and_bare_except() {
    assert_matches_cpython(
        "bridge_exception_bare",
        "pycc_bridge_exc_mod",
        "try:\n    import pycc_nosuch_1293\nexcept Exception:\n    print('exception')\n\
         try:\n    import pycc_nosuch_1293\nexcept:\n    print('bare')\n",
        "exception\nbare\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_try_body_stops_at_the_failed_import() {
    assert_matches_cpython(
        "bridge_stops",
        "pycc_bridge_stops_mod",
        "try:\n    print('a')\n    import pycc_nosuch_1293\n    print('b')\n\
         except ImportError:\n    print('c')\n",
        "a\nc\n",
    );
}

/// Whether `colorsys` was imported is not observable through pycc (T0041,
/// #1289), so only the output is asserted.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_multi_name_import_stops_at_the_first_missing_name() {
    assert_matches_cpython(
        "bridge_multi",
        "pycc_bridge_multi_mod",
        "try:\n    import pycc_nosuch_1293, colorsys\nexcept ImportError:\n    print('c')\n",
        "c\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn except_star_import_error_catches_it() {
    assert_matches_cpython(
        "bridge_star",
        "pycc_bridge_star_mod",
        "try:\n    import pycc_nosuch_1293\nexcept* ImportError:\n    print('star')\n",
        "star\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_failed_import_in_an_except_body_is_caught_by_an_outer_try() {
    assert_matches_cpython(
        "bridge_in_except",
        "pycc_bridge_in_except_mod",
        "try:\n    try:\n        raise ValueError('v')\n    except ValueError:\n\
         \x20       import pycc_nosuch_1293\nexcept ImportError as e:\n    print('outer', e)\n",
        "outer No module named 'pycc_nosuch_1293'\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_failed_import_in_an_else_body_is_caught_by_an_outer_try() {
    assert_matches_cpython(
        "bridge_in_else",
        "pycc_bridge_in_else_mod",
        "try:\n    try:\n        print('body')\n    except ValueError:\n        print('no')\n\
         \x20   else:\n        import pycc_nosuch_1293\nexcept ImportError as e:\n    print('outer', e)\n",
        "body\nouter No module named 'pycc_nosuch_1293'\n",
    );
}

/// The pending `ValueError` is replaced by the import's
/// `ModuleNotFoundError`, which the outer `try` catches, as in CPython.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_failed_import_in_a_finally_body_replaces_the_pending_exception() {
    assert_matches_cpython(
        "bridge_in_finally",
        "pycc_bridge_in_finally_mod",
        "try:\n    try:\n        raise ValueError('v')\n    finally:\n\
         \x20       import pycc_nosuch_1293\nexcept ImportError as e:\n    print('outer', e)\n",
        "outer No module named 'pycc_nosuch_1293'\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_failed_import_in_an_if_inside_a_try_is_caught() {
    assert_matches_cpython(
        "bridge_if_in_try",
        "pycc_bridge_if_in_try_mod",
        "try:\n    if True:\n        import pycc_nosuch_1293\nexcept ImportError:\n    print('caught')\n",
        "caught\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn finally_runs_then_the_original_escapes() {
    let run = run_hosted(
        "bridge_finally_escape",
        "pycc_bridge_fin_mod",
        "try:\n    import pycc_nosuch_1293\nfinally:\n    print('finally')\n",
        &expect_original("pycc_bridge_fin_mod", "pycc_nosuch_1293"),
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "finally\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_unmatched_handler_lets_the_original_escape() {
    let run = run_hosted(
        "bridge_unmatched",
        "pycc_bridge_unmatched_mod",
        "try:\n    import pycc_nosuch_1293\nexcept ValueError:\n    print('wrong')\n",
        &expect_original("pycc_bridge_unmatched_mod", "pycc_nosuch_1293"),
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_bare_reraise_restores_the_original() {
    let run = run_hosted(
        "bridge_reraise",
        "pycc_bridge_reraise_mod",
        "try:\n    import pycc_nosuch_1293\nexcept ImportError:\n    print('h')\n    raise\n",
        &expect_original("pycc_bridge_reraise_mod", "pycc_nosuch_1293"),
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "h\n");
}

/// The fallback-import pattern: a second bridged failure inside the handler
/// must not evict the first one's original.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_second_bridged_failure_does_not_evict_the_first() {
    let run = run_hosted(
        "bridge_second",
        "pycc_bridge_second_mod",
        "try:\n    import pycc_nosuch_1293\nexcept ImportError:\n    try:\n\
         \x20       import pycc_nosuch_1293_b\n    except ImportError:\n        pass\n    raise\n",
        &expect_original("pycc_bridge_second_mod", "pycc_nosuch_1293"),
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "");
}

/// Six swallowed inner failures leave seven live entries, past the bridge
/// table's initial capacity of four, so the table grows while the outer
/// entry is live; the bare `raise` must still find and restore it.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_bridge_table_grows_without_losing_a_live_entry() {
    let inner: String = (0..6)
        .map(|i| {
            format!(
                "    try:\n        import pycc_nosuch_1293_g{i}\n    \
                 except ImportError:\n        pass\n"
            )
        })
        .collect();
    let body =
        format!("try:\n    import pycc_nosuch_1293\nexcept ImportError:\n{inner}    raise\n");
    let run = run_hosted(
        "bridge_grow",
        "pycc_bridge_grow_mod",
        &body,
        &expect_original("pycc_bridge_grow_mod", "pycc_nosuch_1293"),
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "");
}

/// A handler that raises something new replaces the import error: the
/// bridge table misses, and the ordinary rebuild runs.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_new_raise_in_the_handler_replaces_the_import_error() {
    let run = run_hosted(
        "bridge_new_raise",
        "pycc_bridge_new_raise_mod",
        "try:\n    import pycc_nosuch_1293\nexcept ImportError:\n    raise ValueError('x')\n",
        "try:\n\
         \x20   import pycc_bridge_new_raise_mod\n\
         except ValueError as e:\n\
         \x20   assert str(e) == 'x', str(e)\n\
         else:\n\
         \x20   raise AssertionError('the import should have failed')\n",
    );
    assert_ok(&run);
}

/// The recorded `except*` deviation, not intended behaviour: an unmatched
/// `except*` re-raises the exception group pycc wrapped the bridged
/// exception in, whose pointer is not in the bridge table, so the host sees
/// the rebuilt group tag's fallback -- a plain `Exception` carrying the
/// import's message -- rather than CPython's original `ModuleNotFoundError`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_unmatched_except_star_escapes_as_the_recorded_deviation() {
    let run = run_hosted(
        "bridge_star_escape",
        "pycc_bridge_star_escape_mod",
        "try:\n    import pycc_nosuch_1293\nexcept* ValueError:\n    pass\n",
        "try:\n\
         \x20   import pycc_bridge_star_escape_mod\n\
         except BaseException as e:\n\
         \x20   print(type(e).__name__, repr(str(e)))\n",
    );
    assert_ok(&run);
    assert_eq!(
        stdout_of(&run),
        "Exception \"No module named 'pycc_nosuch_1293'\"\n"
    );
}

/// Builds `body` as `module`, then writes the helper module `helper` (whose
/// source is `helper_body`) next to it. The helper must not exist at build
/// time: a sibling `.py` then resolves as a project module, and a nested
/// project import is refused. Written afterwards, it is a foreign import
/// that the host resolves at run time.
fn run_with_post_build_helper(
    tag: &str,
    module: &str,
    body: &str,
    helper: &str,
    helper_body: &str,
    script: &str,
) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    build_ext(&dir, module, body);
    std::fs::write(dir.join(format!("{helper}.py")), helper_body).expect("write the helper");
    python(&dir, script)
}

/// A plain `ImportError` (not a `ModuleNotFoundError`) bridges to tag 26: an
/// inner `except ModuleNotFoundError` does not match it, and an outer
/// `except ImportError` does, with the module's own message.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_plain_import_error_bridges_to_tag_26() {
    let body = "try:\n    try:\n        import pycc_imperr_1293\n    except ModuleNotFoundError:\n\
                \x20       print('wrong')\nexcept ImportError as e:\n    print(e)\n";
    let run = run_with_post_build_helper(
        "bridge_tag_26",
        "pycc_bridge_tag26_mod",
        body,
        "pycc_imperr_1293",
        "raise ImportError('custom')\n",
        "import pycc_bridge_tag26_mod\n",
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "custom\n");
    let dir = ScratchDir::new("bridge_tag_26_oracle").expect("scratch");
    std::fs::write(
        dir.join("pycc_imperr_1293.py"),
        "raise ImportError('custom')\n",
    )
    .expect("write the helper");
    let cpython = oracle()
        .arg(source(&dir, body))
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_ok(&cpython);
    assert_eq!(stdout_of(&cpython), "custom\n");
}

/// Part 1 of #1096: an import that fails with anything other than an
/// `ImportError` is bridged by the object bridge, so `except Exception`
/// runs as it does under CPython and the import completes. This test
/// pinned the old direct `-1` edge until #1096 flipped it.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_non_import_error_is_bridged_by_the_object_bridge() {
    let run = run_with_post_build_helper(
        "bridge_non_import_error",
        "pycc_bridge_non_ie_mod",
        "try:\n    import pycc_raises_1293\nexcept Exception:\n    print('handled')\n",
        "pycc_raises_1293",
        "raise ValueError('boom')\n",
        "import pycc_bridge_non_ie_mod\n",
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "handled\n");
}

/// Builds `body` as a plain (embedded) executable and runs it and CPython on
/// the same source. `msvcrt` is a standard-library root that is absent off
/// Windows, so no `pycc.lock` is needed (#1242).
#[cfg(not(windows))]
fn run_embedded(tag: &str, body: &str) -> (Output, Output) {
    let dir = ScratchDir::new(tag).expect("scratch");
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
    let cpython = oracle()
        .arg(dir.join("m.py"))
        .output()
        .expect("python3 should spawn");
    (embedded, cpython)
}

#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_catches_a_failed_import_like_cpython() {
    let (embedded, cpython) = run_embedded(
        "bridge_embedded_caught",
        "try:\n    import msvcrt\nexcept ImportError as e:\n    print(\"caught\", e)\n\
         finally:\n    print(\"fin\")\n",
    );
    assert_ok(&embedded);
    assert_ok(&cpython);
    assert_eq!(
        stdout_of(&embedded),
        "caught No module named 'msvcrt'\nfin\n"
    );
    assert_eq!(stdout_of(&embedded), stdout_of(&cpython));
}

/// An uncaught failed import keeps the embed spec's text: the final
/// `Type: message` line CPython's traceback ends with, and exit status 1.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_uncaught_failed_import_keeps_the_embed_spec_text() {
    let (embedded, cpython) = run_embedded(
        "bridge_embedded_uncaught",
        "print(\"a\")\nif True:\n    import msvcrt\nprint(\"b\")\n",
    );
    assert_eq!(embedded.status.code(), Some(1), "{}", stderr_of(&embedded));
    assert_eq!(stdout_of(&embedded), "a\n");
    assert_eq!(
        stderr_of(&embedded),
        "ModuleNotFoundError: No module named 'msvcrt'\n"
    );
    assert_eq!(cpython.status.code(), Some(1));
    assert_eq!(stdout_of(&cpython), stdout_of(&embedded));
    assert_eq!(
        stderr_of(&cpython).lines().last(),
        stderr_of(&embedded).lines().last()
    );
}
