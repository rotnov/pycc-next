//! Part 2a of #1371: calling a subscript result (`callbacks[k](x)`) and
//! passing a CPython object as a call argument or a subscript key, in a
//! module body and in a function body.
//!
//! Every hosted test builds the source with `pycc build --ext`, imports the
//! artifact into the host CPython, and compares its output with CPython's
//! own run of the same source. The hosted tests are `#[ignore]`d and
//! contribute no line coverage; the Tier-1 `native-build-test` leg runs them
//! with `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in `crates/pycc_hir/src/tests/subscript_call.rs`,
//! `crates/pycc_types/src/foreign/subscript_call_tests.rs`,
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

/// A sibling project module whose class answers `Reg["x"]` with a CPython
/// object through `__class_getitem__`; the main module binds that result and
/// calls it through a name (a borrowed callee).
const REG: &str = "import builtins\n\
    \n\
    \n\
    class Reg:\n    @staticmethod\n    def __class_getitem__(k: str) -> object:\n        \
    return builtins.len\n";

/// The shapes Part 2a admits. `builtins.__dict__` is the table throughout:
/// indexing it yields a CPython callable, which is exactly lark's
/// `callbacks[rule](...)` shape.
///
/// `pin` repeats the new calls 200 times and compares `len`'s reference
/// count before and after: a consumed borrowed callee would drive it down
/// (and eventually free a builtin), a leaked produced callee up.
const SUCCESS: &str = "import builtins\n\
    import copy\n\
    import sys\n\
    from reg import Reg\n\
    \n\
    callbacks = builtins.__dict__\n\
    ln = builtins.len\n\
    o = callbacks[\"str\"](\"abc\")\n\
    k = callbacks[\"str\"](\"len\")\n\
    print(callbacks[\"len\"](\"abcd\"))\n\
    print(callbacks[\"len\"](o))\n\
    print(callbacks[k](\"xy\"))\n\
    f = callbacks[\"abs\"]\n\
    print(f(-3))\n\
    print(copy.deepcopy(o) == o)\n\
    print(callbacks[\"max\"](1, 2.5, True))\n\
    print(callbacks[\"str\"](callbacks[\"len\"](\"abc\")) == \"3\")\n\
    print(callbacks[\"divmod\"](7, 2)[0])\n\
    r = Reg[\"x\"]\n\
    print(r(\"abcde\"))\n\
    \n\
    \n\
    def run(name: str, s: str) -> None:\n    print(callbacks[name](s))\n\
    \n\
    \n\
    def pin() -> None:\n    g = Reg[\"x\"]\n    before = sys.getrefcount(ln)\n    i = 0\n    \
    while i < 200:\n        callbacks[\"len\"](o)\n        callbacks[k](o)\n        \
    g(\"abc\")\n        i += 1\n    print(sys.getrefcount(ln) == before)\n\
    \n\
    \n\
    def catch() -> None:\n    try:\n        callbacks[\"nope\"](\"x\")\n    \
    except KeyError:\n        print(\"KeyError\")\n    try:\n        callbacks[\"len\"](5)\n    \
    except TypeError:\n        print(\"TypeError\")\n    try:\n        \
    callbacks[\"int\"](\"z\")\n    except ValueError:\n        print(\"ValueError\")\n\
    \n\
    \n\
    run(\"len\", \"hello\")\n\
    run(\"repr\", \"q\")\n\
    pin()\n\
    catch()\n";

/// What `SUCCESS` prints under CPython.
const SUCCESS_OUT: &str = "4\n3\n2\n3\nTrue\n2.5\nTrue\n3\n5\n5\n'q'\nTrue\nKeyError\nTypeError\n\
    ValueError\n";

/// `SUCCESS` without its `reg.py` half passes `pycc check`, a native-mode
/// check (it has no `--ext`, so `reg.py`'s `object` return annotation is
/// for the hosted build only, D-258).
#[test]
fn check_accepts_the_success_program_without_its_sibling() {
    let body = SUCCESS
        .replace("from reg import Reg\n", "")
        .replace("r = Reg[\"x\"]\nprint(r(\"abcde\"))\n", "")
        .replace("    g = Reg[\"x\"]\n", "    g = ln\n");
    assert!(!body.contains("Reg"), "{body}");
    let dir = ScratchDir::new("obj_calls_check").expect("scratch");
    let output = check_with(&dir, &body);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// The shapes Part 2a does not admit keep a diagnostic, never a panic.
#[test]
fn the_shapes_outside_part_2a_are_refused() {
    const HEAD: &str = "import builtins\n\ncallbacks = builtins.__dict__\n";
    for (tag, tail, code, needle) in [
        (
            "obj_calls_list_arg",
            "callbacks[\"len\"]([1])\n",
            "I0404",
            "passing a `list[int]` argument to a CPython object's call",
        ),
        (
            "obj_calls_list_key",
            "callbacks[[1]](1)\n",
            "I0404",
            "indexing a CPython object with a `list[int]` key",
        ),
        (
            "obj_calls_native_table",
            "t = {\"a\": 1}\nt[\"a\"](1)\n",
            "C0001",
            "calling a subscript expression is not supported yet",
        ),
        (
            // A plain keyword argument is admitted since Part 8 of #1371
            // (`tests/issue_1371_object_call_kwargs.rs`); `**` is not.
            "obj_calls_keyword",
            "d = {\"obj\": 1}\ncallbacks[\"len\"](**d)\n",
            "C0001",
            "keyword call arguments are not supported yet",
        ),
    ] {
        assert_one_error(tag, &format!("{HEAD}{tail}"), code, needle);
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

/// Builds `body` (beside `reg.py`), then asserts the extension's import
/// report matches CPython's own run of the same source, and returns it.
fn assert_matches_cpython(tag: &str, module: &str, body: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    write(&dir, "reg.py", REG);
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
fn object_calls_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_calls_hosted", "pycc_obj_calls_mod", SUCCESS);
    assert_eq!(out, format!("{SUCCESS_OUT}no error\n"));
}

/// An error raised by the subscript (a missing key) or by the call itself
/// propagates out of the module body like any other object-call error.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_subscript_call_propagates_from_the_module_body() {
    for (tag, module, tail, raised) in [
        (
            "obj_calls_raise_key",
            "pycc_obj_calls_raise_key",
            "callbacks[\"nope\"](1)\nprint(\"unreached\")\n",
            "KeyError",
        ),
        (
            "obj_calls_raise_call",
            "pycc_obj_calls_raise_call",
            "callbacks[\"len\"](5)\nprint(\"unreached\")\n",
            "TypeError",
        ),
        (
            "obj_calls_raise_arg",
            "pycc_obj_calls_raise_arg",
            "callbacks[\"len\"](callbacks[\"int\"](\"z\"))\nprint(\"unreached\")\n",
            "ValueError",
        ),
    ] {
        let body = format!("import builtins\n\ncallbacks = builtins.__dict__\n{tail}");
        let out = assert_matches_cpython(tag, module, &body);
        assert_eq!(out, format!("{raised}\n"), "{tag}");
    }
}

/// lark `lalr_parser_state.py` lines 64, 88 and 101, each with its other
/// constructs replaced (`.append`, `not in`): an object argument to
/// `deepcopy`/`copy`, `callbacks[token.type](token)` and
/// `callbacks[rule](s)` under a conditional expression.
const LARK_LINES: &str = "from copy import copy, deepcopy\n\
    import builtins\n\
    import types\n\
    \n\
    \n\
    def p64(value_stack: object, deepcopy_values: bool) -> object:\n    \
    return deepcopy(value_stack) if deepcopy_values else copy(value_stack)\n\
    \n\
    \n\
    def p88(token: object, callbacks: object) -> object:\n    \
    return callbacks[token.type](token)\n\
    \n\
    \n\
    def p101(callbacks: object, rule: object, s: object) -> object:\n    \
    value = callbacks[rule](s) if callbacks else s\n    return value\n\
    \n\
    \n\
    vs = builtins.__dict__[\"list\"]()\n\
    print(p64(vs, True) == vs)\n\
    print(p64(vs, False) == vs)\n\
    tok = types.SimpleNamespace()\n\
    builtins.setattr(tok, \"type\", \"name\")\n\
    cbs = builtins.__dict__[\"dict\"]()\n\
    cbs.__setitem__(\"name\", builtins.repr)\n\
    print(p88(tok, cbs))\n\
    rule = builtins.__dict__[\"str\"](\"len\")\n\
    print(p101(builtins.__dict__, rule, builtins.__dict__[\"str\"](\"abcd\")))\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_lark_lines_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_calls_lark", "pycc_obj_calls_lark", LARK_LINES);
    assert_eq!(out, "True\nTrue\nnamespace(type='name')\n4\nno error\n");
}
