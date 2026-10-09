//! Part 1 of #891: tuple-unpacking assignment (`a, b = value`) into bare
//! names, from a native tuple of matching arity (native and `--ext` builds)
//! and from a CPython object (`--ext` only, arity checked at run time).
//!
//! The native tests build and run a program and compare it with CPython's
//! own run. The hosted tests build with `pycc build --ext`, import the
//! artifact into the host CPython, and compare with CPython's run of the
//! same source; they are `#[ignore]`d and contribute no line coverage (the
//! Tier-1 `native-build-test` leg runs them with `--include-ignored`). The
//! changed lines are covered by the unit tests in
//! `crates/pycc_hir/src/stmt/unpack/tests.rs`,
//! `crates/pycc_types/src/unpack/tests.rs`,
//! `crates/pycc_mir/src/tests/unpack.rs` and
//! `crates/pycc_codegen/src/tests/object_unpack.rs`.

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

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
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

/// Native unpacks: a module-level tuple, a parameter, a swap (the value is
/// evaluated before any target is bound), a chain piece, a three-target
/// unpack, and a list-display target (`[g, h] = ...`, the same statement
/// in CPython).
const NATIVE: &str = "def scale(t: tuple[int, float]) -> float:\n    \
    n, f = t\n    \
    return f * n\n\
    \n\
    \n\
    p = (3, 1.5)\n\
    a, b = p\n\
    print(a)\n\
    print(b)\n\
    print(scale((2, 0.25)))\n\
    i, j = 1, 2\n\
    i, j = j, i\n\
    print(i)\n\
    print(j)\n\
    q = c, d = (7, 8)\n\
    print(q[0] + q[1])\n\
    print(c * d)\n\
    x, y, z = (2.5, 4, True)\n\
    print(x)\n\
    print(y)\n\
    print(z)\n\
    [g, h] = (5, 6)\n\
    print(g - h)\n";

const NATIVE_OUT: &str = "3\n1.5\n0.5\n2\n1\n15\n56\n2.5\n4\nTrue\n-1\n";

#[test]
fn a_native_tuple_unpack_runs_like_cpython() {
    let dir = ScratchDir::new("unpack_native").expect("scratch");
    let source = write(&dir, "a.py", NATIVE);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let run = Command::new(dir.join("app"))
        .output()
        .expect("the binary should run");
    assert_ok(&run);
    assert_eq!(stdout_of(&run), NATIVE_OUT);
    let oracle = host_python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert_ok(&oracle);
    assert_eq!(stdout_of(&oracle), NATIVE_OUT);
}

/// The shapes Part 1 does not admit keep a diagnostic, never a panic.
#[test]
fn the_shapes_outside_part_1_are_refused() {
    for (tag, body, code, needle) in [
        (
            "unpack_too_many",
            "t = (1, 2, 3)\na, b = t\n",
            "T0055",
            "too many values to unpack (expected 2, got 3)",
        ),
        (
            "unpack_not_enough",
            "a, b, c = (1, 2)\n",
            "T0055",
            "not enough values to unpack (expected 3, got 2)",
        ),
        (
            "unpack_list_value",
            "xs = [1, 2]\na, b = xs\n",
            "C0001",
            "unpacking a `list[int]` value into names is not supported yet",
        ),
        (
            "unpack_int_value",
            "a, b = 5\n",
            "C0001",
            "unpacking a `int` value into names is not supported yet",
        ),
        ("unpack_starred", "a, *b = (1, 2)\n", "C0001", "starred"),
        (
            "unpack_nested",
            "a, (b, c) = (1, (2, 3))\n",
            "C0001",
            "a nested target (`a, (b, c) = ...`) in a tuple-unpacking assignment is not supported yet",
        ),
        (
            "unpack_subscript_element",
            "d = {\"k\": 1}\nd[\"k\"], b = (1, 2)\n",
            "C0001",
            "only bare-name targets in a tuple-unpacking assignment are supported so far, got a subscript",
        ),
    ] {
        assert_one_error(tag, body, code, needle);
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

/// Runs `target` and prints what it raised, type and message, after
/// whatever the module body printed.
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

/// Object unpacks in a module body and a function body, a native tuple in
/// the same `--ext` build, and the failing unpacks caught in a function.
/// Every message here is the same on CPython 3.13 and 3.14: the
/// version-dependent ", got N" suffix of "too many values" for an exact
/// list, tuple or dict is never printed.
const SUCCESS: &str = "import builtins\n\
    \n\
    pair = builtins.tuple(builtins.range(2))\n\
    u, v = pair\n\
    print(u, v)\n\
    k, w = builtins.dict.fromkeys(builtins.str(\"ab\"))\n\
    print(k, w)\n\
    s1, s2, s3 = builtins.str(\"xyz\")\n\
    print(s3, s2, s1)\n\
    n, t = (4, 0.5)\n\
    print(n, t)\n\
    \n\
    \n\
    def inner(seq: object) -> object:\n    \
    first, second = seq\n    \
    return second\n\
    \n\
    \n\
    def catch() -> None:\n    \
    a = builtins.str(\"kept\")\n    \
    try:\n        a, b = builtins.list(builtins.range(3))\n    \
    except ValueError:\n        print(\"ValueError\")\n    \
    print(a)\n    \
    try:\n        a, b = builtins.iter(builtins.range(1))\n    \
    except ValueError as e:\n        print(e)\n    \
    try:\n        a, b = builtins.iter(builtins.range(5))\n    \
    except ValueError as e:\n        print(e)\n    \
    try:\n        a, b = builtins.len\n    \
    except TypeError as e:\n        print(e)\n\
    \n\
    \n\
    print(inner(builtins.list(\"pq\")))\n\
    catch()\n";

const SUCCESS_OUT: &str = "0 1\na b\nz y x\n4 0.5\nq\nValueError\nkept\n\
    not enough values to unpack (expected 2, got 1)\n\
    too many values to unpack (expected 2)\n\
    cannot unpack non-iterable builtin_function_or_method object\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn object_unpacks_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython("unpack_hosted", "pycc_unpack_mod", SUCCESS);
    assert_eq!(out, format!("{SUCCESS_OUT}no error\n"));
}

/// A failing unpack in a module body propagates out of the import with
/// CPython's exception type and message, and no later statement runs.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_failing_module_body_unpack_propagates_like_cpython() {
    for (tag, module, tail, raised) in [
        (
            "unpack_raise_short",
            "pycc_unpack_raise_short",
            "a, b, c = builtins.str(\"xy\")\nprint(\"unreached\")\n",
            "ValueError not enough values to unpack (expected 3, got 2)",
        ),
        (
            "unpack_raise_long",
            "pycc_unpack_raise_long",
            "a, b = builtins.iter(builtins.range(3))\nprint(\"unreached\")\n",
            "ValueError too many values to unpack (expected 2)",
        ),
        (
            "unpack_raise_type",
            "pycc_unpack_raise_type",
            "a, b = builtins.len\nprint(\"unreached\")\n",
            "TypeError cannot unpack non-iterable builtin_function_or_method object",
        ),
        // A static type whose `tp_name` is dotted: CPython prints the
        // module prefix too.
        (
            "unpack_raise_dotted",
            "pycc_unpack_raise_dotted",
            "a, b = builtins.eval(\"__import__('datetime').date(2000, 1, 1)\")\nprint(\"unreached\")\n",
            "TypeError cannot unpack non-iterable datetime.date object",
        ),
    ] {
        let body = format!("import builtins\n\nprint(\"start\")\n{tail}");
        let out = assert_matches_cpython(tag, module, &body);
        assert_eq!(out, format!("start\n{raised}\n"), "{tag}");
    }
}

/// The documented reference-count behaviour (`docs/RUNTIME.md`), pinned on
/// two distinct mortal objects so an improvement is a visible test change.
/// Each element read leaks one reference (the #1092 leak-only rule for
/// object subscripts). The exact-tuple fast path also keeps one reference
/// to the source tuple per unpack; any other iterable is first collected
/// into a fresh tuple, which leaks and keeps one more reference to each
/// element. CPython's own deltas after the loops are `1 1 0` for both.
/// The third loop pins the failing path: a three-item list (`p1, p2, p1`) unpacked into
/// two names raises `ValueError` every time, and the shim releases the
/// iterator, the partial tuple and the extra item, so nothing leaks.
const REFCOUNT: &str = "import builtins\n\
    import sys\n\
    \n\
    \n\
    def measure() -> None:\n    \
    p1 = builtins.object()\n    \
    p2 = builtins.object()\n    \
    d = builtins.dict.fromkeys(builtins.str(\"ab\"), p1)\n    \
    d.__setitem__(\"b\", p2)\n    \
    pr = builtins.tuple(d.values())\n    \
    ls = builtins.list(d.values())\n    \
    b1 = int(sys.getrefcount(p1))\n    \
    b2 = int(sys.getrefcount(p2))\n    \
    bt = int(sys.getrefcount(pr))\n    \
    bl = int(sys.getrefcount(ls))\n    \
    i = 0\n    \
    while i < 100:\n        e1, e2 = pr\n        i += 1\n    \
    print(int(sys.getrefcount(p1)) - b1, int(sys.getrefcount(p2)) - b2, int(sys.getrefcount(pr)) - bt)\n    \
    b1 = int(sys.getrefcount(p1))\n    \
    b2 = int(sys.getrefcount(p2))\n    \
    i = 0\n    \
    while i < 100:\n        f1, f2 = ls\n        i += 1\n    \
    print(int(sys.getrefcount(p1)) - b1, int(sys.getrefcount(p2)) - b2, int(sys.getrefcount(ls)) - bl)\n    \
    d3 = builtins.dict.fromkeys(builtins.str(\"abc\"), p1)\n    \
    d3.__setitem__(\"b\", p2)\n    \
    l3 = builtins.list(d3.values())\n    \
    b1 = int(sys.getrefcount(p1))\n    \
    b2 = int(sys.getrefcount(p2))\n    \
    bl = int(sys.getrefcount(l3))\n    \
    i = 0\n    \
    while i < 100:\n        \
    try:\n            g1, g2 = l3\n        except ValueError:\n            i += 0\n        \
    i += 1\n    \
    print(int(sys.getrefcount(p1)) - b1, int(sys.getrefcount(p2)) - b2, int(sys.getrefcount(l3)) - bl)\n\
    \n\
    \n\
    measure()\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn object_unpack_reference_counts_are_pinned() {
    let (dir, compiled) = compiled_report("unpack_refcount", "pycc_unpack_rc", REFCOUNT);
    assert_eq!(compiled, "100 100 100\n200 200 0\n0 0 0\nno error\n");
    let oracle = python(&dir, &raised_report("runpy.run_path('m.py')"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&oracle), "1 1 0\n1 1 0\n0 0 0\nno error\n");
}

/// lark `lalr_parser_state.py` line 77 in a function of its own:
/// `action, arg = states[state][token.type]`.
const LARK_LINE: &str = "import builtins\n\
    \n\
    \n\
    def step(states: object, state: int, token: object) -> object:\n    \
    action, arg = states[state][token.type]\n    \
    print(action)\n    \
    return arg\n\
    \n\
    \n\
    states = builtins.eval(\"{0: {'NAME': ('shift', 7)}, 1: {'NAME': ['reduce', 2]}}\")\n\
    token = builtins.eval(\"type('Token', (), {'type': 'NAME'})()\")\n\
    print(step(states, 0, token))\n\
    print(step(states, 1, token))\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_lark_line_behaves_like_cpython_in_the_host() {
    let out = assert_matches_cpython("unpack_lark", "pycc_unpack_lark", LARK_LINE);
    assert_eq!(out, "shift\n7\nreduce\n2\nno error\n");
}
