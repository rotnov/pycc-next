//! End-to-end proof for the shift and bitwise operators ([#1210], Part 2 of
//! [#1018]).
//!
//! `docs/TYPE_SYSTEM.md`'s "Bitwise and shift operators" rule is the
//! contract. The byte-exact oracle fixture is
//! `tests/fixtures/bitwise_shift_ops.py`; this file owns the refusals, the
//! uncaught-exception exit, the plain-value `|` pins and the hosted `--ext`
//! arm.
//!
//! [#1210]: https://github.com/rotnov/pycc/issues/1210
//! [#1018]: https://github.com/rotnov/pycc/issues/1018

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn rendered(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.replace("\r\n", "\n")
}

/// Runs `pycc check` on `source` and returns the rendered diagnostics,
/// asserting that the check failed.
fn check_fails(category: &str, source: &str) -> String {
    let dir = ScratchDir::new(category).expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write the subject");
    let output = pycc()
        .arg("check")
        .arg(&path)
        .output()
        .expect("pycc should spawn");
    assert!(!output.status.success(), "{source} was accepted");
    rendered(&output)
}

/// Builds `source` as a standalone executable and runs it.
fn build_and_run(category: &str, source: &str) -> Output {
    let dir = ScratchDir::new(category).expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write the subject");
    let build = pycc()
        .arg("build")
        .arg(&path)
        .arg("-o")
        .arg(dir.join("subject"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{source}: {}", rendered(&build));
    Command::new(dir.join("subject"))
        .output()
        .expect("the built program should spawn")
}

/// `@` is the one binary operator still without a lowering, in both of its
/// forms, spelled in Python and located on the expression or statement.
#[test]
fn matrix_multiplication_is_refused_in_python_spelling() {
    for (source, header, location) in [
        (
            "x = 1 @ 2\n",
            "error[C0001]: binary operator `@` is not supported yet",
            "1:5",
        ),
        (
            "x = 1\nx @= 2\n",
            "error[C0001]: augmented assignment operator `@=` is not supported yet",
            "2:1",
        ),
    ] {
        let text = check_fails("e2e_1210_matmul", source);
        assert!(text.contains(header), "{source}: {text}");
        assert!(
            text.contains(&format!("subject.py:{location}")),
            "{source}: expected the diagnostic at {location}: {text}"
        );
    }
}

/// The operators are defined on `int` and `bool` only. A set or dict
/// operand of an operator CPython does define for it is a capability gap
/// ("not supported yet"); every other pair is a type error ("not
/// defined").
#[test]
fn a_non_integer_operand_is_a_type_error() {
    for (source, header) in [
        (
            "x = 1.0 << 1\n",
            "error[T0021]: operator `<<` on `float` and `int` is not defined",
        ),
        (
            "x = 1 & 1.5\n",
            "error[T0021]: operator `&` on `int` and `float` is not defined",
        ),
        (
            "x = \"a\" ^ 1\n",
            "error[T0021]: operator `^` on `str` and `int` is not defined",
        ),
        (
            "x = {1} | {2}\n",
            "error[T0021]: operator `|` on `set[int]` and `set[int]` is not supported yet",
        ),
        (
            "s = {1}\ns |= {2}\n",
            "error[T0021]: operator `|` on `set[int]` and `set[int]` is not supported yet",
        ),
        (
            "d1 = {\"a\": 1}\nd2 = {\"b\": 2}\nx = d1 | d2\n",
            "error[T0021]: operator `|` on `dict[str, int]` and `dict[str, int]` is not \
             supported yet",
        ),
        (
            "d1 = {\"a\": 1}\nd2 = {\"b\": 2}\nx = d1 & d2\n",
            "error[T0021]: operator `&` on `dict[str, int]` and `dict[str, int]` is not defined",
        ),
        (
            "x = {1} | 1\n",
            "error[T0021]: operator `|` on `set[int]` and `int` is not defined",
        ),
        (
            "x = {1} << 1\n",
            "error[T0021]: operator `<<` on `set[int]` and `int` is not defined",
        ),
    ] {
        let text = check_fails("e2e_1210_t0021", source);
        assert!(text.contains(header), "{source}: {text}");
    }
}

/// A `bool` name keeps its type through `&=`, `|=` and `^=` of a `bool`,
/// but a shift or an `int` value produces an `int` it cannot hold.
#[test]
fn a_bool_name_cannot_take_an_int_result() {
    for source in ["flag = True\nflag <<= 1\n", "flag = True\nflag &= 1\n"] {
        let text = check_fails("e2e_1210_t0023", source);
        assert!(
            text.contains(
                "error[T0023]: cannot assign `int` to `flag`, previously inferred as `bool`"
            ),
            "{source}: {text}"
        );
    }
}

/// An uncaught shift error ends the program the way CPython's does: the
/// exception line on stderr and exit status 1, after the output before it.
#[test]
fn an_uncaught_negative_shift_count_exits_with_value_error() {
    let run = build_and_run(
        "e2e_1210_uncaught",
        "def f(n: int) -> int:\n    return 1 << n\n\nprint(\"before\")\nprint(f(-1))\n\
         print(\"after\")\n",
    );
    assert_eq!(run.status.code(), Some(1), "{}", rendered(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "before\n");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("ValueError: negative shift count"),
        "{}",
        rendered(&run)
    );
}

/// A plain `READ | WRITE` is a value, at module level and in a function:
/// the PEP 604 `|` of `annotation_to_ty` is reached only from an
/// annotation. Also runs every operator, and the `bool` forms, through the
/// whole pipeline without the oracle.
#[test]
fn a_value_bitwise_or_is_an_integer_expression() {
    let run = build_and_run(
        "e2e_1210_value_or",
        "READ = 4\nWRITE = 2\nFLAGS = READ | WRITE\nprint(FLAGS)\n\n\
         def mode(r: int, w: int) -> int:\n    both = r | w\n    return both\n\n\
         print(mode(READ, WRITE))\n\
         print(FLAGS << 2, FLAGS >> 1, FLAGS & 3, FLAGS ^ 7)\n\
         print(True & False, True | False, True ^ True)\n\
         x: int = True\nprint(x & True, x | 0, x << 1)\n",
    );
    assert!(run.status.success(), "{}", rendered(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "6\n6\n24 3 2 1\nFalse True False\nTrue 1 2\n"
    );
}

const SUBJECT: &str = "\
def mask(a: int, b: int) -> int:
    return (a & b) | (a ^ b)


def shl(a: int, n: int) -> int:
    x = a
    x <<= n
    return x


def shr(a: int, n: int) -> int:
    return a >> n
";

/// The operators called from a real interpreter, with smallint results,
/// and a negative count surfacing as the host's `ValueError`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn hosted_bitwise_exports_match_cpython() {
    let dir = ScratchDir::new("e2e_1210_hosted").expect("scratch");
    let dir: &Path = &dir;
    std::fs::write(dir.join("bit_probe.py"), SUBJECT).expect("write the subject");
    let build = pycc()
        .arg("build")
        .arg(dir.join("bit_probe.py"))
        .arg("-o")
        .arg(dir.join("bit_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import bit_probe\n\
             for a, b in [(12, 10), (-12, 10), (12, -10), (0, 0)]:\n\
             \x20   assert bit_probe.mask(a, b) == (a & b) | (a ^ b), (a, b)\n\
             assert bit_probe.shl(3, 4) == 48\n\
             assert bit_probe.shl(-3, 4) == -48\n\
             assert bit_probe.shr(-49, 4) == -4\n\
             assert bit_probe.shr(49, 100) == 0\n\
             try:\n\
             \x20   bit_probe.shl(1, -1)\n\
             except ValueError as error:\n\
             \x20   assert str(error) == 'negative shift count', str(error)\n\
             else:\n\
             \x20   raise AssertionError('a negative count was accepted')\n\
             print('ok')\n",
        )
        .current_dir(dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", rendered(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n"),
        "ok\n"
    );
}
