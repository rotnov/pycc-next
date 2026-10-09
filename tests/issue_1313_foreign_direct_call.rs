//! #1313: a module-body direct call of a name bound to a CPython object --
//! `from itertools import product` followed by `product("ab")` -- calls the
//! object through CPython's own vectorcall with positional scalar arguments,
//! so the result and any raised exception are CPython's own.
//!
//! Every hosted test compares against the host interpreter's own run of the
//! same source. The hosted tests are `#[ignore]`d and contribute no line
//! coverage; the Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the non-ignored tests here and by the unit tests in
//! `crates/pycc_types/src/foreign/call_tests.rs`,
//! `crates/pycc_mir/src/tests/obj_call.rs` and
//! `crates/pycc_codegen/src/foreign_call/tests/call_tests.rs`.

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

/// Writes `body` to `dir/<file>` and returns the path.
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

/// `pycc check` of `body` fails with exactly one `code` diagnostic whose
/// text contains `needle`.
fn assert_one_error(tag: &str, body: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check_with(&dir, body);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]")), "{rendered}");
    assert!(rendered.contains(needle), "{rendered}");
}

/// The success program the hosted tests run: the from-import form is
/// deliberate: it calls the imported function directly, keeping the program
/// clear of the attribute-form `operator.add` method call that #1095's
/// receiver dispatch handles.
const SUCCESS: &str = "from operator import add, truth\n\
    print(int(add(2, 3)))\n\
    print(str(add(\"a\", \"b\")))\n\
    print(len(add(\"ab\", \"c\")))\n\
    print(bool(truth(0)))\n\
    add(1.5, 2.5)\n";

#[test]
fn check_accepts_a_module_body_direct_call() {
    let dir = ScratchDir::new("obj_call_check").expect("scratch");
    for body in [
        SUCCESS,
        "from itertools import product\nproduct(\"ab\", \"cd\")\nproduct()\n",
        "import itertools\nitertools(\"ab\")\n",
    ] {
        let output = check_with(&dir, body);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{body}{}{}",
            stdout_of(&output),
            stderr_of(&output)
        );
    }
}

/// The admission is by type, not provenance: a `for` loop target bound to
/// a CPython object is callable in the loop body like a foreign binding.
/// `OrderedDict.__mro__` is fixed by CPython itself -- `OrderedDict`,
/// `dict`, `object`, each callable with no arguments -- so the output does
/// not depend on the host's site configuration the way an iterable such
/// as `sys.meta_path` would (a `.pth`-installed finder instance is not
/// callable).
const LOOP_TARGET: &str = "from collections import OrderedDict\n\
    n = 0\n\
    for c in OrderedDict.__mro__:\n    c()\n    n = n + 1\n\
    print(n)\n";

#[test]
fn check_accepts_a_call_of_a_foreign_loop_target() {
    let dir = ScratchDir::new("obj_call_loop_target_check").expect("scratch");
    let output = check_with(&dir, LOOP_TARGET);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// A function body may call the object (#1316) and, since Part 1 of #1333,
/// bind its result to a local, while above its import the name is not yet
/// bound.
#[test]
fn the_callee_follows_the_foreign_object_rules() {
    let dir = ScratchDir::new("obj_call_fn_body").expect("scratch");
    let output = check_with(
        &dir,
        "from itertools import product\n\n\ndef f() -> None:\n    product(\"ab\")\n    p = product(\"ab\")\n",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert_one_error(
        "obj_call_early",
        "product(\"ab\")\nfrom itertools import product\n",
        "T0021",
        "call to undefined function `product`",
    );
}

/// Only positional `int`/`float`/`bool`/`str` arguments are marshalled; the
/// argument shapes outside that keep a refusal.
#[test]
fn the_argument_shapes_outside_the_scalars_are_refused() {
    assert_one_error(
        "obj_call_list_arg",
        "from itertools import product\nproduct([1, 2])\n",
        "I0404",
        "passing a `list[int]` argument to a CPython object's call",
    );
    assert_one_error(
        // A plain keyword argument is admitted since Part 8 of #1371;
        // `**` unpacking is not.
        "obj_call_keyword",
        "from itertools import product\nd = {\"repeat\": 2}\nproduct(\"ab\", **d)\n",
        "C0001",
        "keyword call arguments are not supported yet",
    );
    assert_one_error(
        "obj_call_starred",
        "from itertools import product\nxs = [\"ab\"]\nproduct(*xs)\n",
        "C0001",
        "a starred expression",
    );
}

/// Iterating the call's result is a separate shape and stays refused.
#[test]
fn iterating_a_direct_call_is_still_refused() {
    assert_one_error(
        "obj_call_for",
        "from itertools import product\nfor t in product(\"ab\", \"cd\"):\n    pass\n",
        "C0001",
        "only iterating over `range(...)`",
    );
}

/// A cross-target native build cannot embed the build host's interpreter:
/// one `I0403`, unchanged by this issue.
#[test]
fn a_target_build_of_a_direct_call_is_refused() {
    let dir = ScratchDir::new("obj_call_target").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", SUCCESS))
        .arg("-o")
        .arg(dir.join("app"))
        .args(["--target", "x86_64-apple-darwin"])
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(rendered.contains("error[I0403]"), "{rendered}");
    assert!(rendered.contains("in a `--target` build"), "{rendered}");
}

/// Builds `body` as the extension module `module` in `dir` (the source
/// stays at `dir/m.py`, where the oracle runs it too).
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

/// Runs `target` and prints what it raised -- any exception, by type and
/// `str(e)` -- after whatever the module body printed.
fn raised_report(target: &str) -> String {
    format!(
        "import runpy\n\
         try:\n\
         \x20   {target}\n\
         except Exception as e:\n\
         \x20   print(type(e).__name__, e)\n\
         else:\n\
         \x20   print('no error')\n"
    )
}

/// Builds `body`, then returns the extension's import report and CPython's
/// own run of the same source.
fn compiled_and_oracle(tag: &str, module: &str, body: &str) -> (String, String) {
    let dir = ScratchDir::new(tag).expect("scratch");
    build_ext(&dir, module, body);
    let compiled = python(&dir, &raised_report(&format!("import {module}")));
    assert_ok(&compiled);
    let oracle = python(&dir, &raised_report("runpy.run_path('m.py')"));
    assert_ok(&oracle);
    (stdout_of(&compiled), stdout_of(&oracle))
}

/// As [`compiled_and_oracle`], asserting the two match byte for byte.
fn assert_matches_cpython(tag: &str, module: &str, body: &str) -> String {
    let (compiled, oracle) = compiled_and_oracle(tag, module, body);
    assert_eq!(compiled, oracle);
    compiled
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_direct_call_returns_cpythons_result_in_the_host() {
    let out = assert_matches_cpython("obj_call_hosted", "pycc_obj_call_mod", SUCCESS);
    assert_eq!(out, "5\nab\n3\nFalse\nno error\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_call_of_a_foreign_loop_target_runs_like_cpython_in_the_host() {
    let out = assert_matches_cpython(
        "obj_call_loop_target",
        "pycc_obj_call_loop_target_mod",
        LOOP_TARGET,
    );
    assert_eq!(out, "3\nno error\n");
}

/// A raising call surfaces CPython's own exception from the import.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_direct_call_raises_cpythons_exception_in_the_host() {
    let out = assert_matches_cpython(
        "obj_call_raises",
        "pycc_obj_call_raises_mod",
        "from itertools import product\nproduct(1, 2)\n",
    );
    assert_eq!(out, "TypeError 'int' object is not iterable\n");
    let out = assert_matches_cpython(
        "obj_call_module",
        "pycc_obj_call_module_mod",
        "import itertools\nitertools(\"ab\")\n",
    );
    assert_eq!(out, "TypeError 'module' object is not callable\n");
}

/// Part 1 of #1096: a failed foreign call inside a module-level `try` is
/// bridged to the enclosing handler, so the compiled import prints `caught`
/// and completes exactly as CPython does. This test asserted the old
/// uncatchable edge until #1096 flipped it.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_direct_call_is_catchable_in_the_module_body() {
    let out = assert_matches_cpython(
        "obj_call_try",
        "pycc_obj_call_try_mod",
        "from operator import add\ntry:\n    add(1, \"x\")\nexcept TypeError:\n    print(\"caught\")\n",
    );
    assert_eq!(out, "caught\nno error\n");
}

/// The borrowing entry point takes its own reference on the callee for the
/// call and releases it: calling a module global three times leaves the
/// callable's reference count where importing it alone does. A consuming
/// call would drop the global's reference once per call.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_direct_call_leaves_the_callee_reference_count_unchanged_in_the_host() {
    let dir = ScratchDir::new("obj_call_refcount").expect("scratch");
    let delta = |module: &str, body: &str| -> String {
        build_ext(&dir, module, body);
        let run = python(
            &dir,
            &format!(
                "import sys, operator\n\
                 before = sys.getrefcount(operator.add)\n\
                 import {module}\n\
                 print(sys.getrefcount(operator.add) - before)\n"
            ),
        );
        assert_ok(&run);
        stdout_of(&run)
    };
    let import_only = delta("pycc_obj_call_rc_base_mod", "from operator import add\n");
    let with_calls = delta(
        "pycc_obj_call_rc_calls_mod",
        "from operator import add\nadd(1, 2)\nadd(1, 2)\nadd(1, 2)\n",
    );
    assert_eq!(with_calls, import_only);
}

/// A plain (embedded) build compiles its module with `ext` set, so the
/// direct call runs there too, matching CPython's own output.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_runs_a_direct_call_like_cpython() {
    let dir = ScratchDir::new("obj_call_embedded").expect("scratch");
    let source = write(&dir, "m.py", SUCCESS);
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
    assert_ok(&embedded);
    let oracle = host_python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert_ok(&oracle);
    assert_eq!(stdout_of(&embedded), stdout_of(&oracle));
    assert_eq!(stdout_of(&embedded), "5\nab\n3\nFalse\n");
}
