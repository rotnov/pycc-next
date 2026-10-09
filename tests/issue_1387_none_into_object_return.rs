//! Part 1 of #1387: in a function declared `-> Any`/`-> object`, a bare
//! `return` and `return None` hand the caller CPython's `None`, and in an
//! `--ext` module the unannotated operand of `__eq__`/`__ne__` is the opaque
//! CPython object (`docs/TYPE_SYSTEM.md`, the `object` row; D-258's #1387
//! amendment).
//!
//! The diagnostic tests pin what the change leaves alone. The hosted tests
//! are `#[ignore]`d and contribute no line coverage; the Tier-1
//! `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`, comparing the extension
//! artifact against CPython's own run of the same source.

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

/// Writes `body` to `dir/<file>`.
fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

/// The artifact's file name for `-o <dir>/m`: `.pyd` on Windows and
/// `.abi3.so` elsewhere (`docs/CLI_SPEC.md`).
fn artifact_name() -> &'static str {
    if cfg!(windows) { "m.pyd" } else { "m.abi3.so" }
}

/// `pycc build --ext` (when `ext`) or `pycc check` of `body` as `m.py`.
fn compile(tag: &str, body: &str, ext: bool) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    let source = write(&dir, "m.py", body);
    let mut command = pycc();
    if ext {
        command
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(dir.join("m"))
            .arg("--ext");
    } else {
        command.arg("check").arg(&source);
    }
    command.output().expect("pycc should spawn")
}

/// Asserts `body` compiles.
fn assert_admitted(tag: &str, body: &str, ext: bool) {
    let output = compile(tag, body, ext);
    assert!(
        output.status.success(),
        "{tag}: {}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// Asserts `body` is refused with exactly `expected` among its diagnostics.
fn assert_refused(tag: &str, body: &str, ext: bool, expected: &str) {
    let output = compile(tag, body, ext);
    let rendered = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert_eq!(output.status.code(), Some(1), "{tag}: {rendered}");
    assert!(rendered.contains(expected), "{tag}: {rendered}");
}

/// Since #1475 (Part 2 of #1387) these `--ext` programs box their native
/// value into the `object` slot instead of being refused: a `None`-typed
/// call returned as `Any`, `None` under an `Any` annotation, and an explicit
/// `__eq__` call with an `int` or instance argument.
/// `tests/issue_1475_object_boxing.rs` runs them against CPython. Building
/// an `--ext` artifact needs a CPython 3.13+ to build against, so this test
/// is `#[ignore]`d like every other `--ext` build.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn native_values_box_into_object_slots_under_ext() {
    assert_admitted(
        "1387_none_call",
        "from typing import Any\n\ndef g() -> None:\n    return\n\n\
         def f() -> Any:\n    return g()\n",
        true,
    );
    assert_admitted(
        "1387_ann_assign",
        "from typing import Any\n\ndef f() -> None:\n    y: Any = None\n    print(y)\n",
        true,
    );
    assert_admitted(
        "1387_eq_int_arg",
        &format!("{EQ_CLASS}def run() -> bool:\n    return C(1).__eq__(3)\n"),
        true,
    );
    assert_admitted(
        "1387_eq_instance_arg",
        &format!("{EQ_CLASS}def run() -> bool:\n    return C(1).__eq__(C(2))\n"),
        true,
    );
}

/// A class whose `__eq__` takes an unannotated (`object`) operand.
const EQ_CLASS: &str = "class C:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\
                        \x20   def __eq__(self, other) -> bool:\n        return other is None\n\n\n";

#[test]
fn falling_off_the_end_of_an_object_function_is_still_refused() {
    assert_refused(
        "1387_fall_off",
        "from typing import Any\n\ndef f(x: int) -> Any:\n    if x:\n        return None\n",
        true,
        "error[T0022]: function `f` can exit without returning `object`",
    );
}

/// `None` into an annotated object binding is boxed since #1475
/// ([`native_values_box_into_object_slots_under_ext`]); an `Optional`
/// value, which has no single packer, keeps its `T0025`.
#[test]
fn an_optional_into_an_annotated_object_binding_is_still_refused() {
    assert_refused(
        "1387_ann_assign_optional",
        "from typing import Any\n\ndef f(n: int | None) -> None:\n    y: Any = n\n    print(y)\n",
        true,
        "error[T0025]",
    );
}

#[test]
fn a_native_program_checks_a_none_return_into_a_foreign_class() {
    // The return-position rule keys on the declared type alone, so it holds
    // wherever `object` exists: in a `native` program a foreign class
    // annotates as it (Part 1 of #1367). The embedded runtime links the same
    // `pycc_ext_obj_none` the `--ext` artifact does.
    let output = compile(
        "1387_native_foreign_return",
        "from json import JSONDecoder\n\n\ndef f(x: int) -> JSONDecoder:\n\
         \x20   if x:\n        return\n    return None\n\n\nprint(f(1) is None)\n",
        false,
    );
    assert!(
        output.status.success(),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// The `pycc check` test above pins the type rule; this one builds and runs
/// the same program as an embedded `native` executable, so the codegen path
/// and the embedded runtime's `pycc_ext_obj_none` are exercised too.
#[cfg(not(windows))]
#[test]
#[ignore = "needs a relocatable CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn a_native_executable_returns_none_into_a_foreign_class_return() {
    let dir = ScratchDir::new("1387_native_foreign_run").expect("scratch");
    let source = write(
        &dir,
        "m.py",
        "from json import JSONDecoder\n\n\ndef f(x: int) -> JSONDecoder:\n\
         \x20   if x:\n        return\n    return None\n\n\n\
         print(f(1) is None, f(0) is None)\n",
    );
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc runs");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let run = Command::new(dir.join("app"))
        .env("PYTHONIOENCODING", "utf-8")
        .output()
        .expect("the embedded executable runs");
    assert_eq!(run.status.code(), Some(0), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "True True\n");
}

/// In an `--ext` module the operand is the object, so an in-module explicit
/// `__eq__` call boxes a native argument since #1475 (Part 2 of #1387,
/// [`native_values_box_into_object_slots_under_ext`]). A container argument
/// is still the `T0021` argument mismatch.
#[test]
fn an_explicit_eq_call_with_a_container_argument_is_refused_under_ext() {
    let class = EQ_CLASS;
    assert_refused(
        "1387_eq_list_arg",
        &format!("{class}def run() -> bool:\n    return C(1).__eq__([3])\n"),
        true,
        "error[T0021]: argument 1 of `__eq__` expects `object`, got `list[int]`",
    );
    // The `native` build of the same call still cannot infer the operand.
    assert_refused(
        "1387_eq_int_arg_native",
        &format!("{class}def run() -> bool:\n    return C(1).__eq__(3)\n"),
        false,
        "error[T0021]: cannot infer type of parameter `other`",
    );
}

#[test]
fn a_native_build_keeps_t0021_for_an_unannotated_equality_operand() {
    // `object` is not a type a `native` program can spell, so the operand
    // stays an inference variable with no call site to infer it from.
    assert_refused(
        "1387_native_eq",
        "class C:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\
         \x20   def __eq__(self, other) -> bool:\n        return False\n",
        false,
        "error[T0021]: cannot infer type of parameter `other`",
    );
}

#[test]
fn another_dunder_s_unannotated_operand_keeps_t0021_under_ext() {
    assert_refused(
        "1387_lt",
        "class C:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\
         \x20   def __lt__(self, other) -> bool:\n        return False\n",
        true,
        "error[T0021]: cannot infer type of parameter `other`",
    );
}

// ---------------------------------------------------------------------
// Hosted: the artifact against CPython's own run of the same source.
// ---------------------------------------------------------------------

/// Runs `script` with `dir` as the working directory, stdout forced to UTF-8.
fn host_run(dir: &Path, script: &str) -> Output {
    host_python()
        .args(["-B", "-c", script])
        .current_dir(dir)
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .output()
        .expect("python3 should spawn")
}

/// Builds `body` as the extension `m` into `compiled_dir`, asserting success.
fn build_ext(compiled_dir: &Path, source_dir: &Path, body: &str) {
    let source = write(source_dir, "m.py", body);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(compiled_dir.join("m"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
    assert!(compiled_dir.join(artifact_name()).is_file());
}

/// Builds `body` as the extension `m`, runs `script` against it and against
/// the same source imported as `m.py` by CPython, asserts both succeed with
/// the same stdout, and returns it.
fn assert_matches_cpython(tag: &str, body: &str, script: &str) -> String {
    let compiled_dir = ScratchDir::new(tag).expect("scratch");
    let source_dir = ScratchDir::new(&format!("{tag}_src")).expect("scratch");
    build_ext(&compiled_dir, &source_dir, body);
    let compiled = host_run(&compiled_dir, script);
    let oracle = host_run(&source_dir, script);
    for (what, run) in [("pycc", &compiled), ("cpython", &oracle)] {
        assert!(
            run.status.success(),
            "{what}: {}{}",
            stdout_of(run),
            stderr_of(run)
        );
    }
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    stdout_of(&compiled)
}

/// The #1207 subject's `ParserState.feed_token` control shape (lark
/// `lark/parsers/lalr_parser_state.py`, lines 67-109): an `-> Any` method
/// whose `while True` loop leaves through a bare `return` (line 89) or by
/// returning an object (line 109). The unannotated private helper puts the
/// module through the constraint solver as well as the check phase. The
/// script also returns `None` many times over, which would exhaust a
/// borrowed-reference miscount on a CPython whose `None` is not immortal.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_subject_s_feed_token_shape_matches_cpython() {
    let stdout = assert_matches_cpython(
        "1387_hosted_feed",
        "from typing import Any\n\
         def _start(x):\n    return x\n\
         class ParserState:\n\
         \x20   def __init__(self, stop: int) -> None:\n        self.stop = stop\n\
         \x20   def feed_token(self, values: list[Any], is_end=False) -> Any:\n\
         \x20       i = _start(0)\n\
         \x20       while True:\n\
         \x20           if i >= self.stop:\n                return\n\
         \x20           if is_end:\n                return None\n\
         \x20           if i == 5:\n                return values[i]\n\
         \x20           i = i + 1\n\
         def plain(flag: bool, o: object) -> object:\n\
         \x20   if flag:\n        return None\n\
         \x20   return o\n\
         def guarded(flag: bool, o: object) -> Any:\n\
         \x20   try:\n        if flag:\n            return\n        return o\n\
         \x20   finally:\n        print('finally')\n",
        "import m\n\
         values = list(range(10, 17))\n\
         s = m.ParserState(2)\n\
         print(s.feed_token(values) is None, s.feed_token(values, True) is None)\n\
         print(m.ParserState(9).feed_token(values))\n\
         print(m.plain(True, 'x') is None, m.plain(False, 'x'))\n\
         print(m.guarded(True, 'y') is None)\n\
         print(m.guarded(False, 'y'))\n\
         print(all(m.plain(True, 'x') is None for _ in range(100000)))\n",
    );
    assert_eq!(
        stdout,
        "True True\n15\nTrue x\nfinally\nTrue\nfinally\ny\nTrue\n"
    );
}

/// The subject's `ParserState.__eq__(self, other) -> bool` signature (line
/// 51), unannotated as in lark, and its `__ne__` counterpart. The script
/// reaches them through public module functions that call them explicitly;
/// a host `==` on the instance runs them through the carrier type's
/// `tp_richcompare` since #1427 (`tests/issue_1427_ext_richcompare.rs`).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_subject_s_unannotated_eq_matches_cpython() {
    let stdout = assert_matches_cpython(
        "1387_hosted_eq",
        "class ParserState:\n\
         \x20   def __init__(self, n: int) -> None:\n        self.n = n\n\
         \x20   def __eq__(self, other) -> bool:\n        return other is None\n\
         \x20   def __ne__(self, other) -> bool:\n        return other is not None\n\
         \x20   def size(self) -> int:\n        return self.n\n\
         def eq(o: object) -> bool:\n    return ParserState(1).__eq__(o)\n\
         def ne(o: object) -> bool:\n    return ParserState(2).__ne__(o)\n",
        "import m\n\
         print(m.eq(None), m.eq(3), m.eq('s'))\n\
         print(m.ne(None), m.ne([1]))\n",
    );
    assert_eq!(stdout, "True False False\nFalse True\n");
}
