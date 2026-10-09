//! Part 6 of #1371: `and`/`or` with a CPython object operand, in value
//! context (the node is an `object`, a native operand boxed on the arm that
//! selects it) and in truth context (the object's truth is
//! `PyObject_IsTrue`), in a module body and in a function body.
//!
//! Every hosted test builds the source with `pycc build --ext`, imports the
//! artifact into the host CPython, and compares its output with CPython's
//! own run of the same source. The hosted tests are `#[ignore]`d and
//! contribute no line coverage; the Tier-1 `native-build-test` leg runs them
//! with `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in `crates/pycc_hir/src/boolop/tests.rs`,
//! `crates/pycc_types/src/tests/boolop.rs` and
//! `crates/pycc_codegen/src/tests/object_bool_op.rs`.
//!
//! The helper module `pycc_bo_helper` lives in a `host_only/` subdirectory
//! of the scratch dir rather than beside the fixture source: a `.py` next to
//! the source is resolved as a *project* import, so it would never reach
//! the foreign-object path. Every hosted child gets `host_only` on its
//! `PYTHONPATH`, and `PYTHONIOENCODING=utf-8`.

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

/// The host-only helper. `Flag` logs every truth test of it, so the log
/// shows exactly which operands were tested; `Boom`'s truth test raises;
/// `Sized` is falsy through `__len__`; `side` logs that it was evaluated.
const HELPER: &str = "log = []\n\
    \n\
    \n\
    class Flag:\n    \
    def __init__(self, value, name):\n        self.value = value\n        self.name = name\n\n    \
    def __bool__(self):\n        log.append(self.name)\n        return self.value\n\n    \
    def __repr__(self):\n        return self.name\n\
    \n\
    \n\
    class Boom:\n    \
    def __bool__(self):\n        raise ValueError(\"no truth value\")\n\n    \
    def __repr__(self):\n        return \"boom\"\n\
    \n\
    \n\
    class Sized:\n    \
    def __len__(self):\n        return 0\n\n    \
    def __repr__(self):\n        return \"sized0\"\n\
    \n\
    \n\
    def side(x):\n    log.append(\"side \" + str(x))\n    return x\n\
    \n\
    \n\
    yes = Flag(True, \"yes\")\n\
    no = Flag(False, \"no\")\n\
    empty = []\n\
    zero = 0\n\
    none = None\n\
    boom = Boom()\n\
    sized0 = Sized()\n\
    pair = [1, 2]\n\
    other = [3, 2]\n\
    single = [1]\n\
    two = 2\n";

/// The shapes Part 6 admits: short circuit (the `side` log), falsy objects
/// (`[]`, `0`, `None`, a `__len__` of 0, a `__bool__` returning `False`),
/// native/object mixes in both orders, identity of a selected object, a
/// right operand never truth-tested, truth contexts, a function body, and a
/// raising `__bool__` caught in value and in truth context.
///
/// `pin` repeats the new operations 200 times on *mortal* objects, passing
/// each result straight to `builtins.id` so CPython holds no reference to
/// it afterwards either, and compares reference counts before and after: an
/// object operand pycc retained or released would move its count.
const SUCCESS: &str = "import builtins\n\
    import sys\n\
    import pycc_bo_helper\n\
    \n\
    H = pycc_bo_helper\n\
    print(H.yes or H.side(\"r1\"))\n\
    print(H.no or H.side(\"r2\"))\n\
    print(H.yes and H.side(\"r3\"))\n\
    print(H.no and H.side(\"r4\"))\n\
    print(H.log)\n\
    print(H.empty or 7)\n\
    print(H.zero or \"z\")\n\
    print(H.none or 2.5)\n\
    print(H.sized0 or True)\n\
    print(H.empty and 7)\n\
    print(3 and H.zero)\n\
    print(0 or H.none)\n\
    print(True and H.empty)\n\
    print(False or H.zero)\n\
    print(\"\" or H.sized0)\n\
    print(False and H.yes)\n\
    x = H.none or H.empty\n\
    print(x)\n\
    print((H.yes or 1) is H.yes)\n\
    print(1 and H.boom)\n\
    if H.no or H.yes:\n    print(\"truth\")\n\
    if H.empty and H.side(\"never\"):\n    print(\"unreached\")\nelse:\n    print(\"else\")\n\
    n = 5\n\
    if n > 3 and H.yes:\n    print(\"mixed truth\")\n\
    if not (H.no or H.zero):\n    print(\"not\")\n\
    print(H.log)\n\
    \n\
    \n\
    def body(n: int, s: str) -> None:\n    \
    print(H.no or n)\n    \
    print(H.yes and s)\n    \
    print(n and H.zero)\n    \
    print(s or H.empty)\n    \
    y = H.none or H.side(n)\n    \
    print(y)\n    \
    if n > 3 and H.yes:\n        print(\"fn truth\")\n    \
    try:\n        print(H.boom or 1)\n    except ValueError:\n        print(\"ValueError value\")\n    \
    try:\n        if True and H.boom:\n            print(\"unreached\")\n    \
    except ValueError:\n        print(\"ValueError truth\")\n\
    \n\
    \n\
    def pin() -> None:\n    \
    probe = builtins.object()\n    \
    e = builtins.list()\n    \
    before_p = sys.getrefcount(probe)\n    \
    before_e = sys.getrefcount(e)\n    \
    i = 0\n    \
    while i < 200:\n        \
    builtins.id(probe or 7)\n        \
    builtins.id(probe and e)\n        \
    builtins.id(e or 7)\n        \
    builtins.id(e and 7)\n        \
    builtins.id(7 and probe)\n        \
    builtins.id(0 or e)\n        \
    if e or probe:\n            i += 1\n    \
    print(sys.getrefcount(probe) == before_p)\n    \
    print(sys.getrefcount(e) == before_e)\n\
    \n\
    \n\
    body(5, \"s\")\n\
    body(0, \"\")\n\
    pin()\n";

/// What `SUCCESS` prints under CPython.
const SUCCESS_OUT: &str = "yes\nr2\nr3\nno\n['yes', 'no', 'side r2', 'yes', 'side r3', 'no']\n\
    7\nz\n2.5\nTrue\n[]\n0\nNone\n[]\n0\nsized0\nFalse\n[]\nTrue\nboom\ntruth\nelse\n\
    mixed truth\nnot\n\
    ['yes', 'no', 'side r2', 'yes', 'side r3', 'no', 'yes', 'no', 'yes', 'yes', 'no']\n\
    5\ns\n0\ns\n5\nfn truth\nValueError value\nValueError truth\n\
    0\n\n0\n[]\n0\nValueError value\nValueError truth\n\
    True\nTrue\n";

/// `SUCCESS` passes `pycc check`: every object in it comes from a foreign
/// import.
#[test]
fn check_accepts_the_success_program() {
    let dir = ScratchDir::new("obj_boolop_check").expect("scratch");
    let output = check_with(&dir, SUCCESS);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// The pairings Part 6 does not admit keep a diagnostic, never a panic: an
/// operand with no truth test pycc can run, `None`, and a value pycc cannot
/// box (an `Optional`, a compiled-class instance).
#[test]
fn the_pairings_outside_part_6_are_refused() {
    const HEAD: &str = "import pycc_bo_helper\n\nH = pycc_bo_helper\n";
    for (tag, tail, code, needle) in [
        (
            "obj_boolop_list",
            "print(H.yes or [1])\n",
            "T0021",
            "`or` operand of type `list[int]` has no truth value pycc can test",
        ),
        (
            "obj_boolop_none",
            "print(H.yes and None)\n",
            "T0021",
            "`and` operand `None` has no value pycc can join",
        ),
        (
            "obj_boolop_optional",
            "def f(a: int | None) -> None:\n    print(a or H.yes)\n",
            "I0404",
            "joining a CPython object with a `int | None` value in an `or`",
        ),
        (
            "obj_boolop_instance",
            "class C:\n    def __init__(self) -> None:\n        self.n = 1\n\n\
             def f(c: C) -> None:\n    print(H.yes and c)\n",
            "I0404",
            "joining a CPython object with a `C` value in an `and`",
        ),
    ] {
        assert_one_error(tag, &format!("{HEAD}{tail}"), code, needle);
    }
}

/// Writes the helper module into `dir/host_only`.
fn write_helper(dir: &Path) {
    let helper_dir = dir.join("host_only");
    std::fs::create_dir_all(&helper_dir).expect("create the helper directory");
    std::fs::write(helper_dir.join("pycc_bo_helper.py"), HELPER).expect("write the helper");
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
        .env("PYTHONPATH", dir.join("host_only"))
        .env("PYTHONIOENCODING", "utf-8")
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
    write_helper(&dir);
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
fn and_or_with_objects_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_boolop_hosted", "pycc_obj_boolop_mod", SUCCESS);
    assert_eq!(out, format!("{SUCCESS_OUT}no error\n"));
}

/// A raising `__bool__` propagates out of the module body, in value and in
/// truth context, and nothing after it runs.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_truth_test_propagates_from_the_module_body() {
    for (tag, module, tail) in [
        (
            "obj_boolop_raise_or",
            "pycc_obj_boolop_raise_or",
            "print(H.boom or 1)\nprint(\"unreached\")\n",
        ),
        (
            "obj_boolop_raise_and",
            "pycc_obj_boolop_raise_and",
            "print(1 and H.boom and 2)\nprint(\"unreached\")\n",
        ),
        (
            "obj_boolop_raise_truth",
            "pycc_obj_boolop_raise_truth",
            "if H.no or H.boom:\n    print(\"unreached\")\nprint(\"unreached\")\n",
        ),
    ] {
        let body = format!("import pycc_bo_helper\n\nH = pycc_bo_helper\n{tail}");
        let out = assert_matches_cpython(tag, module, &body);
        assert_eq!(out, "ValueError\n", "{tag}");
    }
}

/// The documented divergence (`docs/RUNTIME.md`): a selected `int` operand
/// outside the packer's range raises `OverflowError` in compiled code where
/// CPython returns the integer (#1040). A discarded one is never boxed, so
/// it raises nothing. The function-body case shows the packer failure is
/// catchable like any other foreign failure.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_selected_out_of_range_int_raises_overflow_error() {
    let body = "import pycc_bo_helper\n\
        \n\
        H = pycc_bo_helper\n\
        n = 2\n\
        big = 9223372036854775807 + n\n\
        print(big and H.zero)\n\
        print(H.no and big)\n\
        \n\
        \n\
        def f(m: int) -> None:\n    \
        try:\n        print(H.zero or m)\n    except OverflowError:\n        \
        print(\"OverflowError\")\n\
        \n\
        \n\
        f(big)\n\
        print(big or H.zero)\n\
        print(\"unreached\")\n";
    let (_dir, out) = compiled_report("obj_boolop_overflow", "pycc_obj_boolop_overflow", body);
    assert_eq!(out, "0\nno\nOverflowError\nOverflowError\n");
}

/// lark `lalr_parser_state.py`: lines 43-44 (`self.s = s or default` in
/// `ParserState.__init__`, the defaults passed explicitly; the real right
/// operands are list displays, outside this part), line 54
/// (`len(a) == len(b) and a[-1] == b[-1]`) and line 108
/// (`is_end and state_stack[-1] == end_state`), each with the surrounding
/// constructs removed.
const LARK_LINES: &str = "import pycc_bo_helper\n\
    \n\
    H = pycc_bo_helper\n\
    \n\
    \n\
    class ParserState:\n    \
    state_stack: list\n    \
    value_stack: list\n\
    \n    \
    def __init__(self, start: object, state_stack=None, value_stack=None):\n        \
    self.state_stack = state_stack or start\n        \
    self.value_stack = value_stack or H.side(0)\n\
    \n\
    \n\
    def p54(a: object, b: object) -> object:\n    \
    return len(a) == len(b) and a[-1] == b[-1]\n\
    \n\
    \n\
    def p108(is_end: bool, state_stack: object, end_state: object) -> bool:\n    \
    if is_end and state_stack[-1] == end_state:\n        return True\n    \
    return False\n\
    \n\
    \n\
    p = ParserState(H.pair, H.none, H.none)\n\
    print(p.state_stack)\n\
    print(p.value_stack)\n\
    q = ParserState(H.pair, H.other, H.single)\n\
    print(q.state_stack)\n\
    print(q.value_stack)\n\
    print(p54(H.pair, H.other))\n\
    print(p54(H.single, H.other))\n\
    print(p108(True, H.pair, H.two))\n\
    print(p108(False, H.pair, H.two))\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_lark_lines_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_boolop_lark", "pycc_obj_boolop_lark", LARK_LINES);
    assert_eq!(
        out,
        "[1, 2]\n0\n[3, 2]\n[1]\nTrue\nFalse\nTrue\nFalse\nno error\n"
    );
}
