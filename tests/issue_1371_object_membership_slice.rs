//! Part 2b of #1371: `item in container` / `item not in container` with a
//! CPython object container, and slicing a CPython object (`o[a:b:c]`, any
//! bound omitted), in a module body and in a function body.
//!
//! Every hosted test builds the source with `pycc build --ext`, imports the
//! artifact into the host CPython, and compares its output with CPython's
//! own run of the same source. The hosted tests are `#[ignore]`d and
//! contribute no line coverage; the Tier-1 `native-build-test` leg runs them
//! with `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in `crates/pycc_hir/src/compare_chain/tests.rs`,
//! `crates/pycc_types/src/foreign/compare/tests.rs`,
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
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
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

/// The shapes Part 2b admits. `builtins.__dict__` is the membership table
/// and `builtins.list("abcdef")` the sliced sequence throughout.
///
/// `pin` repeats the new operations 200 times on *mortal* objects -- a
/// fresh `object()` item and a large `int` bound -- and compares their
/// reference counts before and after: a packed operand the helper failed to
/// consume would drive a count up, one consumed twice would drive it down.
/// It never pins `t[:]` of a tuple or str, which CPython answers with the
/// same object.
///
/// `catch` covers the raising paths: `in` on an object with no membership
/// protocol at all, `str`'s own `__contains__` refusing an `int` item,
/// slicing an unsliceable object, and a dict's `__getitem__` refusing a
/// `slice` key.
const SUCCESS: &str = "import builtins\n\
    import sys\n\
    \n\
    callbacks = builtins.__dict__\n\
    vs = builtins.list(\"abcdef\")\n\
    key = builtins.str(\"len\")\n\
    print(\"len\" in callbacks)\n\
    print(\"nope\" not in callbacks)\n\
    print(key in callbacks)\n\
    print(3 in vs)\n\
    print(\"c\" in vs)\n\
    print(2.5 not in vs)\n\
    print(True in vs)\n\
    print(vs[1:3])\n\
    print(vs[:2])\n\
    print(vs[4:])\n\
    print(vs[::2])\n\
    print(vs[-2:])\n\
    print(vs[:])\n\
    print(vs[1:5:2])\n\
    w = vs[1:4]\n\
    print(len(w))\n\
    n = 2\n\
    print(vs[n:n + 2])\n\
    \n\
    \n\
    def member(name: str, size: int) -> None:\n    \
    print(name in callbacks)\n    \
    print(name not in callbacks)\n    \
    print(vs[-size:])\n    \
    print(vs[:size])\n    \
    if name in callbacks:\n        print(\"yes\")\n    \
    k = builtins.str(name)\n    \
    print(k in callbacks)\n    \
    print(vs[k in callbacks:])\n\
    \n\
    \n\
    def pin() -> None:\n    \
    probe = builtins.object()\n    \
    big = builtins.int(\"100000\")\n    \
    before_p = sys.getrefcount(probe)\n    \
    before_v = sys.getrefcount(vs)\n    \
    before_b = sys.getrefcount(big)\n    \
    i = 0\n    \
    while i < 200:\n        \
    if probe in vs:\n            print(\"hit\")\n        \
    vs[big:]\n        \
    vs[1:big:1]\n        \
    i += 1\n    \
    print(sys.getrefcount(probe) == before_p)\n    \
    print(sys.getrefcount(vs) == before_v)\n    \
    print(sys.getrefcount(big) == before_b)\n\
    \n\
    \n\
    def catch() -> None:\n    \
    try:\n        print(1 in builtins.len)\n    except TypeError:\n        print(\"TypeError\")\n    \
    try:\n        print(1 in builtins.str(\"abc\"))\n    except TypeError:\n        print(\"TypeError\")\n    \
    try:\n        print(builtins.len[1:2])\n    except TypeError:\n        print(\"TypeError\")\n    \
    try:\n        print(callbacks[0:1])\n    except KeyError:\n        print(\"KeyError\")\n\
    \n\
    \n\
    member(\"len\", 2)\n\
    member(\"nope\", 3)\n\
    pin()\n\
    catch()\n";

/// What `SUCCESS` prints under CPython.
const SUCCESS_OUT: &str = "True\nTrue\nTrue\nFalse\nTrue\nTrue\nFalse\n['b', 'c']\n['a', 'b']\n\
    ['e', 'f']\n['a', 'c', 'e']\n['e', 'f']\n['a', 'b', 'c', 'd', 'e', 'f']\n['b', 'd']\n3\n\
    ['c', 'd']\nTrue\nFalse\n['e', 'f']\n['a', 'b']\nyes\nTrue\n['b', 'c', 'd', 'e', 'f']\n\
    False\nTrue\n['d', 'e', 'f']\n['a', 'b', 'c']\nFalse\n['a', 'b', 'c', 'd', 'e', 'f']\nTrue\n\
    True\nTrue\nTypeError\nTypeError\nTypeError\nKeyError\n";

/// `SUCCESS` passes `pycc check`, a native-mode check: every object in it
/// comes from a foreign import, never an `object` annotation.
#[test]
fn check_accepts_the_success_program() {
    let dir = ScratchDir::new("obj_member_check").expect("scratch");
    let output = check_with(&dir, SUCCESS);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// The shapes Part 2b does not admit keep a diagnostic, never a panic.
#[test]
fn the_shapes_outside_part_2b_are_refused() {
    const HEAD: &str = "import builtins\n\ncallbacks = builtins.__dict__\nxs = [1, 2]\n";
    for (tag, tail, code, needle) in [
        (
            "obj_member_list_item",
            "print([1] in callbacks)\n",
            "I0404",
            "testing membership of a `list[int]` value in a CPython object",
        ),
        (
            "obj_member_native_container",
            "print(callbacks in xs)\n",
            "I0404",
            "testing membership of a CPython object in a `list[int]` value",
        ),
        (
            "obj_member_native_pair",
            "print(1 in xs)\n",
            "C0001",
            "comparison operator not supported yet: In",
        ),
        (
            "obj_member_display",
            "print(1 not in [1, 2])\n",
            "C0001",
            "comparison operator not supported yet: NotIn",
        ),
        (
            "obj_member_chain",
            "print(1 in callbacks in callbacks)\n",
            "C0001",
            "comparison operator not supported yet: In",
        ),
        (
            "obj_slice_list_bound",
            "print(callbacks[xs:])\n",
            "I0404",
            "slicing a CPython object with a `list[int]` bound",
        ),
        (
            "obj_slice_none_bound",
            "print(callbacks[None:1])\n",
            "I0404",
            "slicing a CPython object with a `None` bound",
        ),
        // A slice store keeps the HIR's own refusal. A slice `del` was a
        // row here until Part 2c of #1371 admitted it
        // (`tests/issue_1371_object_slice_del.rs`).
        (
            "obj_slice_store",
            "callbacks[1:2] = 3\n",
            "C0001",
            "a slice (`a:b`)",
        ),
    ] {
        assert_one_error(tag, &format!("{HEAD}{tail}"), code, needle);
    }
    // Admitting membership moved the native-pair refusal from HIR lowering
    // (which saw only syntax and had the comparison's span) to the type
    // stage, whose diagnostics carry no span yet: it is reported at 1:1, the
    // same trade Part 1 made for general identity, until Part 5 of #1371
    // gives type-stage diagnostics their source location. Pinned so the
    // restoration is a visible test change.
    let dir = ScratchDir::new("obj_member_native_pair_span").expect("scratch");
    let rendered = stdout_of(&check_with(&dir, &format!("{HEAD}print(1 in xs)\n")));
    assert!(rendered.contains(".py:1:1"), "{rendered}");
    assert!(!rendered.contains(".py:5:"), "{rendered}");
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

/// Builds `body`, imports the extension and returns its report.
fn compiled_report(tag: &str, module: &str, body: &str) -> (ScratchDir, String) {
    let dir = ScratchDir::new(tag).expect("scratch");
    build_ext(&dir, module, body);
    let compiled = python(&dir, &raised_report(&format!("import {module}")));
    assert_ok(&compiled);
    let out = stdout_of(&compiled);
    (dir, out)
}

/// Builds `body`, then asserts the extension's import report matches
/// CPython's own run of the same source, and returns it.
fn assert_matches_cpython(tag: &str, module: &str, body: &str) -> String {
    let (dir, compiled) = compiled_report(tag, module, body);
    let oracle = python(&dir, &raised_report("runpy.run_path('m.py')"));
    assert_ok(&oracle);
    assert_eq!(compiled, stdout_of(&oracle));
    compiled
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn membership_and_slices_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_member_hosted", "pycc_obj_member_mod", SUCCESS);
    assert_eq!(out, format!("{SUCCESS_OUT}no error\n"));
}

/// An error raised by the membership test or by the slice load propagates
/// out of the module body like any other object-operation error.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_membership_or_slice_propagates_from_the_module_body() {
    for (tag, module, tail, raised) in [
        (
            "obj_member_raise_in",
            "pycc_obj_member_raise_in",
            "print(1 in builtins.len)\nprint(\"unreached\")\n",
            "TypeError",
        ),
        (
            "obj_member_raise_not_in",
            "pycc_obj_member_raise_not_in",
            "print(1 not in builtins.str(\"abc\"))\nprint(\"unreached\")\n",
            "TypeError",
        ),
        (
            "obj_member_raise_slice",
            "pycc_obj_member_raise_slice",
            "print(builtins.len[1:])\nprint(\"unreached\")\n",
            "TypeError",
        ),
    ] {
        let body = format!("import builtins\n\n{tail}");
        let out = assert_matches_cpython(tag, module, &body);
        assert_eq!(out, format!("{raised}\n"), "{tag}");
    }
}

/// The documented divergence (`docs/RUNTIME.md`): an `int` bound or item
/// outside the packer's range raises `OverflowError` in compiled code,
/// exactly as an `int` subscript key does (#1040), where CPython slices or
/// tests it normally.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_out_of_range_int_bound_or_item_raises_overflow_error() {
    let body = "import builtins\n\
        \n\
        vs = builtins.list(\"abcdef\")\n\
        n = 2\n\
        try:\n    print(vs[9223372036854775807 * n:])\nexcept OverflowError:\n    \
        print(\"OverflowError\")\n\
        try:\n    print(9223372036854775807 * n in vs)\nexcept OverflowError:\n    \
        print(\"OverflowError\")\n";
    let (_dir, out) = compiled_report("obj_member_overflow", "pycc_obj_member_overflow", body);
    assert_eq!(out, "OverflowError\nOverflowError\nno error\n");
}

/// lark `lalr_parser_state.py` lines 88 and 95, each in a function of its
/// own with the surrounding constructs removed: `name not in callbacks`
/// and `value_stack[-size:]`.
const LARK_LINES: &str = "import builtins\n\
    \n\
    \n\
    def p88(name: object, callbacks: object) -> bool:\n    \
    return name not in callbacks\n\
    \n\
    \n\
    def p95(value_stack: object, size: int) -> object:\n    \
    s = value_stack[-size:]\n    return s\n\
    \n\
    \n\
    print(p88(builtins.str(\"len\"), builtins.__dict__))\n\
    print(p88(builtins.str(\"nope\"), builtins.__dict__))\n\
    print(p95(builtins.list(\"abcdef\"), 2))\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_lark_lines_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_member_lark", "pycc_obj_member_lark", LARK_LINES);
    assert_eq!(out, "False\nTrue\n['e', 'f']\nno error\n");
}
