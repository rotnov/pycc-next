//! Builtin functions and builtin-type methods that lower to runtime calls
//! (`pycc_hir::builtins`), `input()`, and the source-level desugarings
//! (`pycc_ast::desugar`), each fixture compared with the live CPython
//! oracle (`PYCC_PYTHON`, default `python3`): stdout byte-for-byte, then
//! the uncaught exception's last stderr line and the exit status. A
//! fixture `tests/fixtures/<name>.py` with a sibling `<name>.stdin` reads
//! that file as its standard input.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_stderr_line(output: &Output) -> String {
    text(&output.stderr)
        .lines()
        .last()
        .unwrap_or_default()
        .to_string()
}

fn run_with_stdin(command: &mut Command, stdin: Option<&Path>) -> Output {
    match stdin {
        Some(path) => command.stdin(std::fs::File::open(path).expect("stdin fixture opens")),
        None => command.stdin(Stdio::null()),
    };
    command.output().expect("process spawns")
}

/// Builds `tests/fixtures/<name>.py`, runs it and CPython on the same
/// input, and compares them.
fn assert_matches_cpython(name: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture = root.join(format!("tests/fixtures/{name}.py"));
    let stdin = root.join(format!("tests/fixtures/{name}.stdin"));
    let stdin = stdin.exists().then_some(stdin.as_path());
    let dir = ScratchDir::new(name).expect("scratch");
    let binary = dir.join("app");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&fixture)
        .arg("-o")
        .arg(&binary)
        .arg("--release")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "pycc build of {name} failed:\n{}{}",
        text(&build.stdout),
        text(&build.stderr)
    );
    let run = run_with_stdin(&mut Command::new(&binary), stdin);
    let oracle = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let cpython = run_with_stdin(Command::new(oracle).arg(&fixture), stdin);
    assert_eq!(text(&run.stdout), text(&cpython.stdout), "{name}: stdout");
    assert_eq!(
        last_stderr_line(&run),
        last_stderr_line(&cpython),
        "{name}: last stderr line"
    );
    assert_eq!(run.status.code(), cpython.status.code(), "{name}: exit status");
}

#[test]
fn str_builtins_and_methods_match_cpython() {
    assert_matches_cpython("builtins_str");
}

#[test]
fn numeric_and_list_builtins_match_cpython() {
    assert_matches_cpython("builtins_num");
}

#[test]
fn input_reads_stdin_lines_and_raises_eof_error_like_cpython() {
    assert_matches_cpython("builtins_input");
}

#[test]
fn desugared_loops_and_stores_match_cpython() {
    assert_matches_cpython("desugar_stmts");
}

#[test]
fn desugared_sys_streams_lambdas_map_and_print_match_cpython() {
    assert_matches_cpython("desugar_io");
}

#[test]
fn str_iteration_membership_and_computed_receivers_match_cpython() {
    assert_matches_cpython("str_loops");
}

/// `int(str)` refuses more than 4300 decimal digits before converting
/// (CPython's default `sys.get_int_max_str_digits()`), so a huge input
/// line cannot drive the quadratic conversion; power-of-two bases are
/// exempt, as in CPython.
#[test]
fn int_of_a_str_enforces_cpythons_digit_limit() {
    assert_matches_cpython("int_str_digit_limit");
}

/// `math.gcd`/`lcm`/`isqrt`/`floor`/`ceil`, through `import math` and
/// `from math import ...`, with bigints and their exceptions.
#[test]
fn math_integer_functions_match_cpython() {
    assert_matches_cpython("math_int_funcs");
}

/// `print`/`str`/f-strings of a `dict[str, int]` and `str()` of a list.
#[test]
fn dict_and_list_repr_match_cpython() {
    assert_matches_cpython("dict_print");
}
