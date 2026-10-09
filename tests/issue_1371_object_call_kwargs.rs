//! Part 8 of #1371: keyword arguments on a call of a CPython object --
//! `o.method(x, key=v)`, `f(a, b=c)` with `f` an `object`, and
//! `table[k](x, key=v)` -- and a `None` call argument, in a module body and
//! in a function body.
//!
//! Every hosted test builds the source with `pycc build --ext`, imports the
//! artifact into the host CPython, and compares its output with CPython's
//! own run of the same source. The hosted tests are `#[ignore]`d and
//! contribute no line coverage; the Tier-1 `native-build-test` leg runs them
//! with `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in
//! `crates/pycc_hir/src/expr/object_keyword_call_tests.rs`,
//! `crates/pycc_types/src/foreign/keyword_call_tests.rs`,
//! `crates/pycc_mir/src/tests/obj_keyword_call.rs` and
//! `crates/pycc_codegen/src/foreign_call/tests/keyword_tests.rs`.

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

const KEYWORD: &str = "keyword call arguments are not supported yet";

/// The module-body shapes Part 8 admits, every one of which `pycc check`
/// (a native-mode check, so no `object` annotation) accepts too.
const MODULE_BODY: &str = "import builtins\n\
    from collections import OrderedDict\n\
    from itertools import product\n\
    \n\
    srt = builtins.sorted\n\
    print(srt(\"312\", reverse=True))\n\
    print(builtins.sorted(\"cab\", key=None, reverse=False))\n\
    print(builtins.list(product(\"ab\", repeat=2)))\n\
    print(OrderedDict(a=1, b=\"x\", c=2.5, d=None))\n\
    print(builtins.int(\"ff\", base=16))\n\
    s = builtins.str(\"a,b,c\")\n\
    print(s.split(\",\", maxsplit=1))\n\
    print(builtins.__dict__[\"int\"](\"11\", base=2))\n\
    print(builtins.max(1, 3, key=None))\n\
    print(builtins.repr(None))\n\
    print(builtins.sorted(s, reverse=srt(\"\") == builtins.list()))\n";

/// What `MODULE_BODY` prints under CPython.
const MODULE_BODY_OUT: &str = "['3', '2', '1']\n['a', 'b', 'c']\n\
    [('a', 'a'), ('a', 'b'), ('b', 'a'), ('b', 'b')]\n\
    OrderedDict({'a': 1, 'b': 'x', 'c': 2.5, 'd': None})\n255\n['a', 'b,c']\n3\n3\nNone\n\
    ['c', 'b', 'a', ',', ',']\n";

/// `MODULE_BODY` plus the function-body shapes: an `object` parameter as a
/// keyword receiver and value, a refcount pin, and the two raised errors.
///
/// `pin` repeats a borrowed-callee keyword call and a consumed-bound-method
/// one 200 times and compares `sorted`'s and the receiver's reference
/// counts before and after: a consumed borrowed callee would drive the
/// first down, a leaked bound method or name tuple the second up.
///
/// `catch` checks an unexpected keyword (CPython's own `TypeError`) and the
/// evaluation order: `s.missing(k=1 // 0)` looks the method up before the
/// keyword value is evaluated, so it raises `AttributeError`, not
/// `ZeroDivisionError`.
const HOSTED_TAIL: &str = "import sys\n\
    \n\
    \n\
    def run(o: object, n: int) -> None:\n    print(o.split(\",\", maxsplit=n))\n    \
    print(srt(o, reverse=n == 0, key=None))\n\
    \n\
    \n\
    def pin(o: object) -> None:\n    before = sys.getrefcount(srt)\n    \
    mine = sys.getrefcount(o)\n    i = 0\n    while i < 200:\n        \
    srt(\"ba\", reverse=True)\n        o.split(\",\", maxsplit=1)\n        i += 1\n    \
    print(sys.getrefcount(srt) == before, sys.getrefcount(o) == mine)\n\
    \n\
    \n\
    def catch() -> None:\n    try:\n        srt(\"1\", nope=1)\n    \
    except TypeError:\n        print(\"TypeError\")\n    try:\n        \
    s.missing(k=1 // 0)\n    except AttributeError:\n        print(\"AttributeError\")\n\
    \n\
    \n\
    run(s, 0)\n\
    pin(s)\n\
    catch()\n";

const HOSTED_TAIL_OUT: &str = "['a,b,c']\n['c', 'b', 'a', ',', ',']\nTrue True\n\
    TypeError\nAttributeError\n";

#[test]
fn check_accepts_every_module_body_keyword_call() {
    let dir = ScratchDir::new("obj_kw_check").expect("scratch");
    let output = check_with(&dir, MODULE_BODY);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// The shapes Part 8 does not admit keep a diagnostic, never a panic.
#[test]
fn the_shapes_outside_part_8_are_refused() {
    const HEAD: &str = "import builtins\nfrom itertools import product\n";
    for (tag, tail, code, needle) in [
        (
            "obj_kw_list_value",
            "product(\"ab\", repeat=[1])\n",
            "I0404",
            "passing a `list[int]` argument to a CPython object's call",
        ),
        (
            // An instance keyword value is admitted since #1435
            // (`tests/issue_1435_instance_argument.rs`); a class method's
            // `cls`, which holds no instance, is not.
            "obj_kw_classmethod_cls",
            "class P:\n    @classmethod\n    def f(cls) -> None:\n        product(state=cls)\n",
            "I0404",
            "passing a class method's `cls` argument to a CPython object's call",
        ),
        (
            "obj_kw_double_star",
            "d = {\"repeat\": 2}\nproduct(\"ab\", **d)\n",
            "C0001",
            KEYWORD,
        ),
        (
            "obj_kw_container_method",
            "d = builtins.dict()\nd.get(\"a\", default=None)\n",
            "C0001",
            KEYWORD,
        ),
        (
            "obj_kw_native_callee",
            "print(1, end=\"\")\n",
            "C0001",
            KEYWORD,
        ),
        (
            "obj_kw_duplicate",
            "product(repeat=1, repeat=2)\n",
            "L0001",
            "",
        ),
    ] {
        assert_one_error(tag, &format!("{HEAD}{tail}"), code, needle);
    }
}

/// A keyword call of a pycc enum class keeps exactly the one keyword
/// `C0001`: the enum-call path no longer reports its own, and the deferral
/// does not add a second one.
#[test]
fn an_enum_keyword_call_keeps_exactly_one_c0001() {
    assert_one_error(
        "obj_kw_enum",
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\nprint(Color(value=1))\n",
        "C0001",
        KEYWORD,
    );
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

/// Builds `body` (beside the pure-Python `lexc.py`), then asserts the
/// extension's import report matches CPython's own run of the same source,
/// and returns it.
fn assert_matches_cpython(tag: &str, module: &str, body: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    write(&dir, "lexc.py", LEXC);
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
fn keyword_calls_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython(
        "obj_kw_hosted",
        "pycc_obj_kw_mod",
        &format!("{MODULE_BODY}{HOSTED_TAIL}"),
    );
    assert_eq!(out, format!("{MODULE_BODY_OUT}{HOSTED_TAIL_OUT}no error\n"));
}

/// An error raised by a keyword call propagates out of the module body like
/// any other object-call error, and the evaluation order holds there too.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_keyword_call_propagates_from_the_module_body() {
    for (tag, module, tail, raised) in [
        (
            "obj_kw_raise_type",
            "pycc_obj_kw_raise_type",
            "builtins.sorted(\"ab\", nope=1)\nprint(\"unreached\")\n",
            "TypeError",
        ),
        (
            "obj_kw_raise_order",
            "pycc_obj_kw_raise_order",
            "builtins.missing(k=1 // 0)\nprint(\"unreached\")\n",
            "AttributeError",
        ),
        (
            "obj_kw_raise_value",
            "pycc_obj_kw_raise_value",
            "builtins.int(\"z\", base=10)\nprint(\"unreached\")\n",
            "ValueError",
        ),
    ] {
        let body = format!("import builtins\n\n{tail}");
        let out = assert_matches_cpython(tag, module, &body);
        assert_eq!(out, format!("{raised}\n"), "{tag}");
    }
}

/// A pure-Python stand-in for lark's `UnexpectedToken`, imported at run
/// time through `importlib` so the build treats it as a CPython object.
const LEXC: &str = "class UnexpectedToken(Exception):\n    \
    def __init__(self, token, expected, state=None, interactive_parser=None):\n        \
    super().__init__(token)\n        self.token = token\n        \
    self.expected = expected\n        self.state = state\n        \
    self.interactive_parser = interactive_parser\n";

/// lark `lalr_parser_state.py` line 80,
/// `raise UnexpectedToken(token, expected, state=self, interactive_parser=None)`,
/// reduced: the `raise` of an object is a later part of #1371 and `self` is
/// a pycc instance (`I0404`, `the_shapes_outside_part_8_are_refused`), so
/// the call is returned and `state` is an object parameter.
const LARK_LINE_80: &str = "import builtins\n\
    import importlib\n\
    \n\
    UnexpectedToken = importlib.import_module(\"lexc\").UnexpectedToken\n\
    \n\
    \n\
    def p80(token: object, expected: object, state: object) -> object:\n    \
    return UnexpectedToken(token, expected, state=state, interactive_parser=None)\n\
    \n\
    \n\
    e = p80(builtins.str(\"tok\"), builtins.dict(), builtins.str(\"st\"))\n\
    print(e.token)\n\
    print(e.expected)\n\
    print(e.state)\n\
    print(e.interactive_parser is None)\n\
    print(builtins.isinstance(e, builtins.Exception))\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn lark_line_80_reduced_behaves_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_kw_lark80", "pycc_obj_kw_lark80", LARK_LINE_80);
    assert_eq!(out, "tok\n{}\nst\nTrue\nTrue\nno error\n");
}
