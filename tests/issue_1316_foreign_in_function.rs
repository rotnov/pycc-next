//! #1316: a module-level foreign name used inside a function body.
//!
//! Every operation a module body admits on a CPython object is admitted in
//! a function, a method and an unannotated private helper too. A failure
//! inside a function is bridged to a pycc exception and branches to the
//! innermost handler, or out of the function when none matches; when it
//! escapes to the host unchanged, the host sees CPython's *original*
//! exception object. A read that runs before the module's `import` raises
//! CPython's own `NameError`.
//!
//! Each case builds `m.py` as an extension and runs a host script against
//! it, then runs the same script against the same source imported by
//! CPython itself, and asserts both runs print the same stdout. Helper
//! modules sit on a host-only `PYTHONPATH` and never print. Both runs use
//! `PYTHONUNBUFFERED=1`: pycc writes straight to the file descriptor while
//! CPython buffers a piped stdout, so interleaved output would otherwise
//! reorder.
//!
//! Every test here is `#[ignore]`d and contributes no line coverage; the
//! Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The Rust lines this
//! change adds are covered by `crates/pycc_codegen/src/foreign_fail.rs`'s
//! unit tests and the runtime and type-layer unit tests.

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

/// Runs `script` against `body` built by pycc as `module`, and against
/// `body` imported as `module` by CPython, and asserts both print
/// `expected`.
fn assert_matches_cpython(
    tag: &str,
    module: &str,
    body: &str,
    helpers: &[(&str, &str)],
    script: &str,
    expected: &str,
) {
    let hosted = ScratchDir::new(tag).expect("scratch");
    write_helpers(&hosted, helpers);
    build_ext(&hosted, module, body);
    let run = python(&hosted, script);
    assert_ok(&run, "pycc");
    assert_eq!(stdout_of(&run), expected, "pycc on {body}");

    let reference = ScratchDir::new(&format!("{tag}_cpython")).expect("scratch");
    write_helpers(&reference, helpers);
    std::fs::write(reference.join(format!("{module}.py")), body).expect("write the oracle source");
    let cpython = python(&reference, script);
    assert_ok(&cpython, "CPython");
    assert_eq!(stdout_of(&cpython), expected, "CPython on {body}");
}

/// The issue's own example.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_issue_example_reads_a_from_import_in_a_function() {
    assert_matches_cpython(
        "fn1316_issue",
        "pycc_fn1316_issue",
        "from copy import copy\n\n\ndef name() -> str:\n    return str(copy.__name__)\n\n\nprint(name())\n",
        &[],
        "import pycc_fn1316_issue\n",
        "copy\n",
    );
}

/// A method call, `len` of an attribute, a subscript load and a truth test
/// in condition position, all inside one function.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn every_admitted_operation_runs_in_a_function() {
    assert_matches_cpython(
        "fn1316_ops",
        "pycc_fn1316_ops",
        "import copy\n\n\ndef f() -> int:\n    n = int(copy.copy(3))\n    k = len(copy.__name__)\n    \
         c = str(copy.__name__[0])\n    if copy.__name__:\n        print(c)\n    return n + k\n",
        &[],
        "import pycc_fn1316_ops\nprint(pycc_fn1316_ops.f())\n",
        "c\n7\n",
    );
}

/// `int("copy")` raises `ValueError`, which the function's own handler
/// catches.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_value_error_is_caught_inside_the_function() {
    assert_matches_cpython(
        "fn1316_value_error",
        "pycc_fn1316_ve",
        "import copy\n\n\ndef f() -> int:\n    try:\n        return int(copy.__name__)\n    \
         except ValueError:\n        return -1\n",
        &[],
        "import pycc_fn1316_ve\nprint(pycc_fn1316_ve.f())\n",
        "-1\n",
    );
}

/// A missing attribute is caught by `except Exception`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_attribute_is_caught_by_except_exception() {
    assert_matches_cpython(
        "fn1316_attr_error",
        "pycc_fn1316_ae",
        "import copy\n\n\ndef f() -> int:\n    try:\n        copy.nope\n        return 0\n    \
         except Exception:\n        return 1\n",
        &[],
        "import pycc_fn1316_ae\nprint(pycc_fn1316_ae.f())\n",
        "1\n",
    );
}

/// The bridge maps `BrokenPipeError` to its own class, not to its
/// `ConnectionError` base, so the more specific handler wins.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_subclass_reaches_its_own_handler_before_its_base() {
    assert_matches_cpython(
        "fn1316_tag_order",
        "pycc_fn1316_bp",
        "import hh\n\n\ndef f() -> int:\n    try:\n        hh.bp()\n        return 0\n    \
         except BrokenPipeError:\n        return 19\n    except ConnectionError:\n        return 10\n",
        &[("hh", "def bp():\n    raise BrokenPipeError('x')\n")],
        "import pycc_fn1316_bp\nprint(pycc_fn1316_bp.f())\n",
        "19\n",
    );
}

/// An uncaught failure in a function the host calls after import reaches
/// the host as CPython's original exception, with its `.name`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_uncaught_failure_reaches_the_host_as_the_original_exception() {
    assert_matches_cpython(
        "fn1316_host_call",
        "pycc_fn1316_hc",
        "import copy\n\n\ndef f() -> int:\n    copy.nope\n    return 0\n",
        &[],
        "import pycc_fn1316_hc\n\
         try:\n\
         \x20   pycc_fn1316_hc.f()\n\
         except AttributeError as e:\n\
         \x20   print(type(e) is AttributeError, e.name)\n\
         print(pycc_fn1316_hc.f.__name__)\n",
        "True nope\nf\n",
    );
}

/// The same failure raised while the module body runs fails the import.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_uncaught_failure_during_module_exec_fails_the_import() {
    assert_matches_cpython(
        "fn1316_exec",
        "pycc_fn1316_ex",
        "import copy\n\n\ndef f() -> int:\n    copy.nope\n    return 0\n\n\nf()\n",
        &[],
        "try:\n\
         \x20   import pycc_fn1316_ex\n\
         except AttributeError as e:\n\
         \x20   print(type(e) is AttributeError, e.name)\n",
        "True nope\n",
    );
}

/// A read that runs before the `import` raises CPython's own `NameError`,
/// uncaught and caught by a bare `except`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_read_before_the_import_raises_name_error() {
    assert_matches_cpython(
        "fn1316_before",
        "pycc_fn1316_nb",
        "def f() -> int:\n    copy.__name__\n    return 0\n\n\nf()\nimport copy\n",
        &[],
        "try:\n\
         \x20   import pycc_fn1316_nb\n\
         except NameError as e:\n\
         \x20   print(type(e).__name__, e)\n",
        "NameError name 'copy' is not defined\n",
    );
    assert_matches_cpython(
        "fn1316_before_caught",
        "pycc_fn1316_nc",
        "def f() -> int:\n    try:\n        copy.__name__\n        return 0\n    except:\n        \
         return 1\n\n\nprint(f())\nimport copy\n",
        &[],
        "import pycc_fn1316_nc\n",
        "1\n",
    );
}

/// A block import whose arm has not run yet: the name is Definitely bound
/// after the `if`/`else`, but `f()` runs before the taken arm's import.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_block_import_not_yet_run_raises_name_error() {
    assert_matches_cpython(
        "fn1316_block",
        "pycc_fn1316_bl",
        "def f() -> int:\n    copy.__name__\n    return 0\n\n\nflag = True\nif flag:\n    f()\n    \
         import copy\nelse:\n    import copy\n",
        &[],
        "try:\n\
         \x20   import pycc_fn1316_bl\n\
         except NameError as e:\n\
         \x20   print(type(e).__name__, e)\n",
        "NameError name 'copy' is not defined\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_method_body_reads_a_foreign_global() {
    assert_matches_cpython(
        "fn1316_method",
        "pycc_fn1316_me",
        "import copy\n\n\nclass A:\n    def m(self) -> str:\n        return str(copy.__name__)\n\n\n\
         print(A().m())\n",
        &[],
        "import pycc_fn1316_me\nprint(pycc_fn1316_me.A().m())\n",
        "copy\ncopy\n",
    );
}

/// The solver path: an unannotated private helper.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_unannotated_helper_reads_a_foreign_global() {
    assert_matches_cpython(
        "fn1316_helper",
        "pycc_fn1316_hp",
        "import copy\n\n\ndef _ident():\n    return len(copy.__name__)\n\n\nprint(_ident())\n",
        &[],
        "import pycc_fn1316_hp\n",
        "4\n",
    );
}

/// Bridged exceptions caught inside a function do not accumulate across
/// host calls: the per-call watermark releases them when the wrapper
/// returns. Counted with `gc`, which tracks a `ValueError` held only by a
/// C strong reference.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn caught_bridged_exceptions_do_not_accumulate_across_calls() {
    assert_matches_cpython(
        "fn1316_watermark",
        "pycc_fn1316_wm",
        "import copy\n\n\ndef spin(n: int) -> int:\n    caught = 0\n    for i in range(n):\n        \
         try:\n            int(copy.__name__)\n        except ValueError:\n            \
         caught += 1\n    return caught\n",
        &[],
        "import gc\n\
         import pycc_fn1316_wm\n\
         counts = []\n\
         for _ in range(5):\n\
         \x20   assert pycc_fn1316_wm.spin(1000) == 1000\n\
         \x20   gc.collect()\n\
         \x20   counts.append(sum(isinstance(o, ValueError) for o in gc.get_objects()))\n\
         print(len(set(counts)) == 1, counts[0] < 1000)\n",
        "True True\n",
    );
}

/// The bridge table is per thread. Thread T2 enters `g`, whose wrapper
/// takes its mark before anything is bridged, and parks. The main thread
/// then calls `f`, which bridges a failure and, in its handler, lets T2
/// finish -- so T2's wrapper releases to its own mark -- before the bare
/// `raise` re-raises. A process-global table would have dropped `f`'s
/// entry and handed the host a rebuilt exception with no `.name`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_second_thread_does_not_release_another_threads_exception() {
    let hh = "import threading\n\
              entered = threading.Event()\n\
              go = threading.Event()\n\
              t = None\n\
              def park():\n\
              \x20   entered.set()\n\
              \x20   go.wait()\n\
              def release():\n\
              \x20   go.set()\n\
              \x20   t.join()\n";
    assert_matches_cpython(
        "fn1316_threads",
        "pycc_fn1316_th",
        "import copy\nimport hh\n\n\ndef g() -> int:\n    hh.park()\n    return 1\n\n\n\
         def f() -> int:\n    try:\n        copy.nope\n    except Exception:\n        \
         hh.release()\n        raise\n    return 0\n",
        &[("hh", hh)],
        "import threading\n\
         import hh\n\
         import pycc_fn1316_th\n\
         res = []\n\
         hh.t = threading.Thread(target=lambda: res.append(pycc_fn1316_th.g()))\n\
         hh.t.start()\n\
         hh.entered.wait()\n\
         try:\n\
         \x20   pycc_fn1316_th.f()\n\
         except AttributeError as e:\n\
         \x20   print(type(e) is AttributeError, e.name)\n\
         print(res)\n",
        "True nope\n[1]\n",
    );
}

/// `SystemExit` is a `BaseException`: `except Exception` does not catch
/// it, a bare `except` does, and the host receives the original object.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_base_exception_is_not_caught_by_except_exception() {
    assert_matches_cpython(
        "fn1316_system_exit",
        "pycc_fn1316_se",
        "import sys\n\n\ndef f() -> int:\n    try:\n        sys.exit(3)\n    except Exception:\n        \
         return 1\n    return 0\n\n\ndef g() -> int:\n    try:\n        sys.exit(3)\n    except:\n        \
         return 2\n    return 0\n",
        &[],
        "import pycc_fn1316_se\n\
         print(pycc_fn1316_se.g())\n\
         try:\n\
         \x20   pycc_fn1316_se.f()\n\
         except SystemExit as e:\n\
         \x20   print(type(e) is SystemExit, e.code)\n",
        "2\nTrue 3\n",
    );
}
