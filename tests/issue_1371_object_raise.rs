//! Part 9 of #1371: `raise o` with a CPython object `o`, in a module body
//! and in a function body -- lark `lalr_parser_state.py` line 80,
//! `raise UnexpectedToken(token, expected, state=self, interactive_parser=None)`.
//!
//! What is raised is decided the way CPython's own `raise` decides it: an
//! exception instance is raised as it is, an exception class is
//! instantiated, and anything else raises `TypeError`. The raise takes the
//! pycc exception path, so `try`/`except` with `Exception` or a builtin
//! class catches it, and an exception that escapes reaches the host as the
//! original CPython object.
//!
//! Every hosted test builds the source with `pycc build --ext`, imports the
//! artifact into the host CPython, and compares its output with CPython's
//! own run of the same source; one more runs a D-248 embedded executable.
//! The hosted tests are `#[ignore]`d and contribute no line coverage; the
//! Tier-1 `native-build-test` leg runs them with `cargo test --workspace --
//! --include-ignored`. The changed lines are covered by the unit tests in
//! `crates/pycc_types/src/foreign/raise_tests.rs`,
//! `crates/pycc_mir/src/tests/obj_raise.rs` and
//! `crates/pycc_codegen/src/tests/object_raise.rs`.

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

/// A sibling module of exception classes, standing in for lark's
/// `lark.exceptions`: `Boom` takes lark's two positional arguments, and
/// `Odd` is a class whose constructor does not return an instance.
const ERRS: &str = "class Boom(Exception):\n    \
    def __init__(self, token, expected, state=None, interactive_parser=None):\n        \
    super().__init__(f\"unexpected {token!r}, expected {sorted(expected)}\")\n        \
    self.state = state\n\
    \n\
    \n\
    class Odd(Exception):\n    \
    def __new__(cls):\n        \
    return 5\n\
    \n\
    \n\
    class Loud(Exception):\n    \
    def __init__(self):\n        \
    raise KeyError(\"from the constructor\")\n\
    \n\
    \n\
    SENTINEL = Boom(\"t\", {\"b\"})\n";

/// Every operand shape, caught inside the compiled code: an instance, a
/// class, a non-exception, a class whose constructor raises, a class whose
/// constructor returns something else, a raise in a loop and in a nested
/// handler, and lark's shape with a computed argument.
const SUCCESS: &str = "import builtins\n\
    from errs import Boom, Odd, Loud\n\
    \n\
    \n\
    def instance(token: str) -> None:\n    \
    raise Boom(token, builtins.set(\"ab\"))\n\
    \n\
    \n\
    def klass() -> None:\n    \
    raise builtins.KeyError\n\
    \n\
    \n\
    def not_an_exception() -> None:\n    \
    raise builtins.int(3)\n\
    \n\
    \n\
    def loud() -> None:\n    \
    raise Loud\n\
    \n\
    \n\
    def odd() -> None:\n    \
    raise Odd\n\
    \n\
    \n\
    def looping(n: int) -> None:\n    \
    while n > 0:\n        \
    n -= 1\n        \
    if n == 1:\n            \
    raise builtins.ValueError(\"in a loop\")\n\
    \n\
    \n\
    def nested() -> None:\n    \
    try:\n        \
    raise builtins.ValueError(\"first\")\n    \
    except ValueError:\n        \
    raise builtins.TypeError(\"second\")\n\
    \n\
    \n\
    try:\n    instance(\"x\")\nexcept Exception:\n    print(\"instance\")\n\
    try:\n    klass()\nexcept KeyError:\n    print(\"class\")\n\
    try:\n    not_an_exception()\nexcept TypeError:\n    print(\"not an exception\")\n\
    try:\n    loud()\nexcept KeyError:\n    print(\"loud\")\n\
    try:\n    odd()\nexcept TypeError:\n    print(\"odd\")\n\
    try:\n    looping(3)\nexcept ValueError:\n    print(\"loop\")\n\
    try:\n    nested()\nexcept TypeError:\n    print(\"nested\")\n\
    try:\n    raise builtins.ValueError(\"module\")\nexcept ValueError:\n    print(\"module\")\n\
    print(\"done\")\n";

const SUCCESS_OUT: &str = "instance\nclass\nnot an exception\nloud\nodd\nloop\nnested\n\
    module\ndone\n";

/// `SUCCESS` passes `pycc check`, a native-mode check: every object in it
/// comes from a foreign import, never an `object` annotation.
#[test]
fn check_accepts_the_success_program() {
    let dir = ScratchDir::new("obj_raise_check").expect("scratch");
    let output = check_with(&dir, SUCCESS);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// A cause next to an object is the deferred `raise ... from`, a native
/// non-exception keeps its `T0021`, and an `except` naming a foreign class
/// is out of scope and keeps its `T0021`.
#[test]
fn the_shapes_outside_part_9_are_refused() {
    const HEAD: &str = "from errs import Boom\n\n";
    for (tag, tail, code, needle) in [
        (
            "obj_raise_from",
            "raise Boom(1, 2) from ValueError(\"c\")\n",
            "C0001",
            "`raise ... from ...` with a CPython object as the exception or the cause is not supported yet",
        ),
        (
            "obj_raise_native_int",
            "raise 1\n",
            "T0021",
            "can only raise exception instances",
        ),
        (
            "obj_raise_except_foreign",
            "try:\n    raise Boom(1, 2)\nexcept Boom:\n    pass\n",
            "T0021",
            "`Boom` is not a recognized exception class",
        ),
    ] {
        assert_one_error(tag, &format!("{HEAD}{tail}"), code, needle);
    }
}

/// `errs.py` lives in `lib/`, on the host's `PYTHONPATH` only: next to
/// `m.py` it would be a project module that `pycc build` compiles too.
fn build_ext(dir: &Path, module: &str, body: &str) {
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib/");
    write(&lib, "errs.py", ERRS);
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
        .env("PYTHONPATH", dir.join("lib"))
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

/// Runs `target` and prints what escaped it -- type, message, and whether
/// it is `errs.SENTINEL`, the one pre-built instance -- after whatever the
/// module body printed.
fn raised_report(target: &str) -> String {
    format!(
        "import runpy\n\
         import sys\n\
         try:\n\
         \x20   {target}\n\
         except BaseException as e:\n\
         \x20   print(type(e).__name__, str(e), e is getattr(sys.modules.get('errs'), 'SENTINEL', None))\n\
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
fn every_operand_shape_behaves_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_raise_hosted", "pycc_obj_raise_mod", SUCCESS);
    assert_eq!(out, format!("{SUCCESS_OUT}no error\n"));
}

/// An object raise that nothing catches escapes the module body as the
/// original CPython object -- its attributes intact -- and the statement
/// after it never runs. `SystemExit` is not an `Exception`, so an
/// `except Exception` does not catch it.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_uncaught_object_raise_escapes_as_the_original_object() {
    for (tag, module, tail, escaped) in [
        (
            "obj_raise_escape_instance",
            "pycc_obj_raise_escape_instance",
            "from errs import SENTINEL\nraise SENTINEL\nprint(\"unreached\")\n",
            "Boom unexpected 't', expected ['b'] True",
        ),
        (
            "obj_raise_escape_function",
            "pycc_obj_raise_escape_function",
            "def f() -> None:\n    raise Boom(\"u\", builtins.set())\n\n\nf()\nprint(\"unreached\")\n",
            "Boom unexpected 'u', expected [] False",
        ),
        (
            "obj_raise_escape_type_error",
            "pycc_obj_raise_escape_type_error",
            "raise builtins.str(\"s\")\n",
            "TypeError exceptions must derive from BaseException False",
        ),
        (
            "obj_raise_escape_system_exit",
            "pycc_obj_raise_escape_system_exit",
            "try:\n    raise builtins.SystemExit(3)\nexcept Exception:\n    print(\"wrong\")\n",
            "SystemExit 3 False",
        ),
        (
            "obj_raise_escape_param",
            "pycc_obj_raise_escape_param",
            "def f(o: object) -> int:\n    raise o\n\n\nf(builtins.ValueError(\"p\"))\n",
            "ValueError p False",
        ),
    ] {
        let body = format!("import builtins\nfrom errs import Boom\n\n{tail}");
        let out = assert_matches_cpython(tag, module, &body);
        assert_eq!(out, format!("{escaped}\n"), "{tag}");
    }
}

/// An object raised while the host is handling an exception gets that
/// exception as its implicit `__context__`, as CPython's `raise` sets it:
/// an instance, an instantiated class, and the `TypeError` for a
/// non-exception alike.
const CONTEXT: &str = "import builtins\n\
    from errs import Boom\n\
    \n\
    \n\
    def raise_instance() -> None:\n    \
    raise Boom(\"c\", builtins.set())\n\
    \n\
    \n\
    def raise_class() -> None:\n    \
    raise builtins.ValueError\n\
    \n\
    \n\
    def raise_other() -> None:\n    \
    raise builtins.str(\"s\")\n";

fn context_report(setup: &str) -> String {
    format!(
        "import runpy\n\
         {setup}\n\
         for name in ('raise_instance', 'raise_class', 'raise_other'):\n\
         \x20   try:\n\
         \x20       raise KeyError('host')\n\
         \x20   except KeyError:\n\
         \x20       try:\n\
         \x20           ns[name]()\n\
         \x20       except BaseException as e:\n\
         \x20           print(type(e).__name__, type(e.__context__).__name__)\n"
    )
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_object_raised_inside_a_host_handler_chains_its_context_like_cpython() {
    let dir = ScratchDir::new("obj_raise_context").expect("scratch");
    build_ext(&dir, "pycc_obj_raise_context", CONTEXT);
    let compiled = python(
        &dir,
        &context_report("import pycc_obj_raise_context\nns = vars(pycc_obj_raise_context)"),
    );
    assert_ok(&compiled);
    let oracle = python(&dir, &context_report("ns = runpy.run_path('m.py')"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(
        stdout_of(&compiled),
        "Boom KeyError\nValueError KeyError\nTypeError KeyError\n"
    );
}

/// lark `lalr_parser_state.py` line 80 in a function of its own, with only
/// `state=self` (a pycc instance, still `I0404`) replaced by `None`:
/// `raise UnexpectedToken(token, expected, state=None,
/// interactive_parser=None)`, spelled `Boom` here, on an `object`-annotated
/// token and a computed set. The keyword call is Part 8's.
const LARK_LINE: &str = "import builtins\n\
    from errs import Boom\n\
    \n\
    \n\
    def feed_token(token: object, states: object) -> None:\n    \
    expected = builtins.set(states)\n    \
    raise Boom(token, expected, state=None, interactive_parser=None)\n\
    \n\
    \n\
    feed_token(builtins.str(\"NAME\"), builtins.list(\"ba\"))\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_lark_line_behaves_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_raise_lark", "pycc_obj_raise_lark", LARK_LINE);
    assert_eq!(out, "Boom unexpected 'NAME', expected ['a', 'b'] False\n");
}

/// A D-248 embedded executable shares the codegen: a caught object raise
/// and an uncaught one behave as under CPython, down to the exit status
/// and the traceback's last line.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_raises_an_object_like_cpython() {
    let dir = ScratchDir::new("obj_raise_embedded").expect("scratch");
    let source = write(
        &dir,
        "m.py",
        "import builtins\n\
         \n\
         try:\n    raise builtins.ValueError(\"caught\")\nexcept ValueError:\n    print(\"caught\")\n\
         raise builtins.KeyError(\"escaped\")\n",
    );
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
    let oracle = host_python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert_eq!(stdout_of(&embedded), "caught\n");
    assert_eq!(stdout_of(&embedded), stdout_of(&oracle));
    assert_eq!(embedded.status.code(), oracle.status.code());
    let last_line = |output: &Output| {
        stderr_of(output)
            .trim_end()
            .lines()
            .last()
            .map(str::to_owned)
    };
    assert_eq!(last_line(&embedded), Some("KeyError: 'escaped'".to_owned()));
    assert_eq!(last_line(&embedded), last_line(&oracle));
}
