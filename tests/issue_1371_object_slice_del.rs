//! Part 2c of #1371: deleting a slice of a CPython object
//! (`del o[a:b:c]`, any bound omitted), in a module body and in a function
//! body -- lark `lalr_parser_state.py` lines 96-97.
//!
//! Every hosted test builds the source with `pycc build --ext`, imports the
//! artifact into the host CPython, and compares its output with CPython's
//! own run of the same source. The hosted tests are `#[ignore]`d and
//! contribute no line coverage; the Tier-1 `native-build-test` leg runs them
//! with `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in `crates/pycc_hir/src/stmt/del/tests.rs`,
//! `crates/pycc_types/src/foreign/slice/tests.rs`,
//! `crates/pycc_mir/src/obj_compare/tests.rs` and
//! `crates/pycc_codegen/src/tests/object_membership_slice.rs`.

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

fn check_with(dir: &Path, body: &str) -> Output {
    pycc()
        .arg("check")
        .arg(write(dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

/// `pycc check` of `body` fails with exactly one `code` diagnostic whose
/// text contains `needle`; returns the rendered diagnostics.
fn assert_one_error(tag: &str, body: &str, code: &str, needle: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check_with(&dir, body);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]")), "{rendered}");
    assert!(rendered.contains(needle), "{rendered}");
    rendered
}

/// The shapes Part 2c admits. `builtins.list("abcdefgh")` is the sequence
/// throughout; each `del` is followed by a `print` of what is left.
///
/// `shrink` is the lark shape: a computed negative bound in a function
/// body, on a module-level object. `order` checks that the bounds are
/// evaluated before the deletion, so a raising bound leaves the list
/// untouched.
///
/// `pin` repeats the deletion 200 times on *mortal* objects -- a fresh
/// list and a large `int` bound -- and compares their reference counts
/// before and after: a packed bound the helper failed to consume would
/// drive a count up, one consumed twice would drive it down.
///
/// `catch` covers the raising paths: a `tuple`'s missing `__delitem__`, an
/// object with no item protocol at all, and a `dict`'s `__delitem__`
/// refusing a `slice` key (hashable since 3.12).
const SUCCESS: &str = "import builtins\n\
    import sys\n\
    \n\
    vs = builtins.list(\"abcdefgh\")\n\
    del vs[7:]\n\
    print(vs)\n\
    del vs[:1]\n\
    print(vs)\n\
    del vs[::2]\n\
    print(vs)\n\
    n = 1\n\
    del vs[n:n + 1]\n\
    print(vs)\n\
    del vs[True:]\n\
    print(vs)\n\
    ws = builtins.list(\"abcdef\")\n\
    del ws[:]\n\
    print(ws)\n\
    \n\
    \n\
    def shrink(size: int) -> None:\n    \
    xs = builtins.list(\"abcdef\")\n    \
    del xs[-size:]\n    \
    print(xs)\n    \
    del xs[1:-1:2]\n    \
    print(xs)\n\
    \n\
    \n\
    def order() -> None:\n    \
    xs = builtins.list(\"abc\")\n    \
    zero = 0\n    \
    try:\n        del xs[1 // zero:]\n    except ZeroDivisionError:\n        \
    print(\"ZeroDivisionError\")\n    \
    print(xs)\n\
    \n\
    \n\
    def pin() -> None:\n    \
    big = builtins.int(\"100000\")\n    \
    ys = builtins.list(\"ab\")\n    \
    before_y = sys.getrefcount(ys)\n    \
    before_b = sys.getrefcount(big)\n    \
    i = 0\n    \
    while i < 200:\n        \
    del ys[big:]\n        \
    del ys[1:big:1]\n        \
    i += 1\n    \
    print(ys)\n    \
    print(sys.getrefcount(ys) == before_y)\n    \
    print(sys.getrefcount(big) == before_b)\n\
    \n\
    \n\
    def catch() -> None:\n    \
    t = builtins.tuple(\"abc\")\n    \
    try:\n        del t[0:1]\n    except TypeError:\n        print(\"TypeError\")\n    \
    try:\n        del builtins.len[1:2]\n    except TypeError:\n        print(\"TypeError\")\n    \
    d = builtins.dict()\n    \
    try:\n        del d[0:1]\n    except KeyError:\n        print(\"KeyError\")\n    \
    print(t)\n\
    \n\
    \n\
    shrink(2)\n\
    shrink(4)\n\
    order()\n\
    pin()\n\
    catch()\n";

/// What `SUCCESS` prints under CPython.
const SUCCESS_OUT: &str = "['a', 'b', 'c', 'd', 'e', 'f', 'g']\n['b', 'c', 'd', 'e', 'f', 'g']\n\
    ['c', 'e', 'g']\n['c', 'g']\n['c']\n[]\n['a', 'b', 'c', 'd']\n['a', 'c', 'd']\n\
    ['a', 'b']\n['a', 'b']\nZeroDivisionError\n['a', 'b', 'c']\n['a']\nTrue\nTrue\n\
    TypeError\nTypeError\nKeyError\n('a', 'b', 'c')\n";

/// `SUCCESS` passes `pycc check`, a native-mode check: every object in it
/// comes from a foreign import, never an `object` annotation.
#[test]
fn check_accepts_the_success_program() {
    let dir = ScratchDir::new("obj_del_slice_check").expect("scratch");
    let output = check_with(&dir, SUCCESS);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// A slice `del` of anything but a CPython object keeps its `C0001`, now
/// located at the slice target; an unpackable bound is the load's own
/// `I0404`; a walrus in a bound and a non-slice subscript keep the HIR's
/// refusals.
#[test]
fn the_shapes_outside_part_2c_are_refused() {
    const HEAD: &str = "import builtins\n\ncallbacks = builtins.__dict__\nxs = [1, 2]\n";
    for (tag, tail, code, needle, location) in [
        (
            "obj_del_slice_native",
            "del xs[0:1]\n",
            "C0001",
            "a `del` of a slice (`del xs[a:b]`) is not supported yet",
            Some(".py:5:5"),
        ),
        (
            "obj_del_slice_list_bound",
            "del callbacks[xs:]\n",
            "I0404",
            "slicing a CPython object with a `list[int]` bound",
            None,
        ),
        (
            "obj_del_slice_walrus",
            "del callbacks[(n := 1):]\n",
            "C0001",
            "a walrus assignment",
            Some(".py:5:5"),
        ),
        (
            "obj_del_subscript",
            "del callbacks[1]\n",
            "C0001",
            "a `del` of a subscript",
            Some(".py:5:5"),
        ),
    ] {
        let rendered = assert_one_error(tag, &format!("{HEAD}{tail}"), code, needle);
        if let Some(location) = location {
            assert!(rendered.contains(location), "{tag}: {rendered}");
        }
    }
}

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

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn slice_deletion_behaves_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_del_slice_hosted", "pycc_obj_del_slice_mod", SUCCESS);
    assert_eq!(out, format!("{SUCCESS_OUT}no error\n"));
}

/// An error raised by the deletion propagates out of the module body like
/// any other object-operation error, and the statement after it never
/// runs.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_slice_deletion_propagates_from_the_module_body() {
    for (tag, module, tail, raised) in [
        (
            "obj_del_slice_raise_type",
            "pycc_obj_del_slice_raise_type",
            "del builtins.len[1:]\nprint(\"unreached\")\n",
            "TypeError",
        ),
        (
            "obj_del_slice_raise_key",
            "pycc_obj_del_slice_raise_key",
            "d = builtins.dict()\ndel d[:]\nprint(\"unreached\")\n",
            "KeyError",
        ),
    ] {
        let body = format!("import builtins\n\n{tail}");
        let out = assert_matches_cpython(tag, module, &body);
        assert_eq!(out, format!("{raised}\n"), "{tag}");
    }
}

/// lark `lalr_parser_state.py` lines 96-97, in a function of its own with
/// the surrounding constructs removed: `del state_stack[-size:]` and
/// `del value_stack[-size:]` on `object`-annotated parameters.
const LARK_LINES: &str = "import builtins\n\
    \n\
    \n\
    def p96(state_stack: object, value_stack: object, size: int) -> None:\n    \
    del state_stack[-size:]\n    \
    del value_stack[-size:]\n\
    \n\
    \n\
    states = builtins.list(\"abcdef\")\n\
    values = builtins.list(\"uvwxyz\")\n\
    p96(states, values, 2)\n\
    print(states)\n\
    print(values)\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_lark_lines_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_del_slice_lark", "pycc_obj_del_slice_lark", LARK_LINES);
    assert_eq!(
        out,
        "['a', 'b', 'c', 'd']\n['u', 'v', 'w', 'x']\nno error\n"
    );
}
