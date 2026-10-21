//! Oracle tests for arbitrary-precision `int` arithmetic: the compiled
//! program (native) and the compiled module (`--ext`) must print exactly
//! what CPython 3.13 prints for the same source, including values around the
//! 63-bit smallint boundary, the `i64` boundary, and far beyond both.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn stdout(output: &Output) -> String {
    pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)).replace("\r\n", "\n")
}

fn rendered(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout(output),
        pycc_scratch::lf(&*String::from_utf8_lossy(&output.stderr))
    )
}

/// Builds `source` natively in `dir` and runs it.
fn build_and_run(dir: &Path, source: &Path) -> Output {
    let binary = dir.join("prog");
    let build = pycc()
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(&binary)
        .arg("--release")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    Command::new(&binary)
        .output()
        .expect("the program should spawn")
}

#[test]
fn bigint_arithmetic_matches_cpython() {
    let dir = ScratchDir::new("int_bigint_native").expect("scratch");
    let source = fixture("int_bigint.py");
    let compiled = build_and_run(&dir, &source);
    let oracle = python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert!(compiled.status.success(), "{}", rendered(&compiled));
    assert_eq!(stdout(&compiled), stdout(&oracle));
}

/// An uncaught error raised by bigint arithmetic ends the program like
/// CPython: exit status 1 and the same exception line.
#[test]
fn an_uncaught_bigint_error_matches_cpython() {
    for (name, body, last_line) in [
        (
            "zero_div",
            "x = 2**100\nprint(x + 1)\nprint(x // (x - x))\nprint('unreachable')\n",
            "ZeroDivisionError: integer division or modulo by zero",
        ),
        (
            "float_overflow",
            "x = 10**400\nprint(x > 0)\nprint(x * 1.5)\nprint('unreachable')\n",
            "OverflowError: int too large to convert to float",
        ),
    ] {
        let dir = ScratchDir::new(&format!("int_bigint_{name}")).expect("scratch");
        let source = dir.join(format!("{name}.py"));
        std::fs::write(&source, body).expect("write the fixture");
        let compiled = build_and_run(&dir, &source);
        let oracle = python()
            .arg(&source)
            .output()
            .expect("python3 should spawn");
        assert_eq!(compiled.status.code(), Some(1), "{}", rendered(&compiled));
        assert_eq!(oracle.status.code(), Some(1), "{}", rendered(&oracle));
        assert_eq!(stdout(&compiled), stdout(&oracle), "{name}");
        let stderr = pycc_scratch::lf(&*String::from_utf8_lossy(&compiled.stderr));
        assert_eq!(
            stderr.trim_end().lines().last(),
            Some(last_line),
            "{stderr}"
        );
        let oracle_stderr = pycc_scratch::lf(&*String::from_utf8_lossy(&oracle.stderr));
        assert_eq!(oracle_stderr.trim_end().lines().last(), Some(last_line));
    }
}

/// A literal too large for a list index is still refused at compile time
/// (`T0051`), now that it lowers to an expansion instead of one `i64`; a
/// container element holds it (v0.6).
#[test]
fn an_oversized_literal_in_a_boundary_position_is_still_t0051() {
    let dir = ScratchDir::new("int_bigint_t0051").expect("scratch");
    let source = dir.join("boundary.py");
    std::fs::write(&source, "xs = [1]\nprint(xs[99999999999999999999999])\n").expect("write");
    let check = pycc()
        .arg("check")
        .arg(&source)
        .output()
        .expect("pycc should spawn");
    assert!(!check.status.success());
    assert!(
        rendered(&check).contains("error[T0051]"),
        "{}",
        rendered(&check)
    );
}

const EXT_PROGRAM: &str = "\
import int_bigint_ext as m
print(m.factorial(60))
print(m.fib(200))
for x in (2**62 - 1, 2**62, -2**62, -2**62 - 1, 2**63, -2**63 - 1, 3**100, -(7**90)):
    print(m.scale(x, 3), m.scale(x, -x), m.floor_pair(x, 7), m.floor_pair(x, -(10**20)))
    print(m.shifted(x, 70), m.mixed(x, 0.5) == x / 3 + 0.5, m.is_huge(x))
print(m.scale(True, 2**70), m.floor_pair(-(10**30), True))
try:
    m.floor_pair(2**100, 0)
except ZeroDivisionError as e:
    print('ZeroDivisionError', e)
try:
    m.mixed(10**400, 0.0)
except OverflowError as e:
    print('OverflowError', e)
";

/// The same functions, compiled as an extension module and imported by a
/// real interpreter, against the `.py` source imported by that interpreter.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn bigint_values_cross_the_ext_boundary_like_cpython() {
    let dir = ScratchDir::new("int_bigint_ext").expect("scratch");
    let build = pycc()
        .arg("build")
        .arg(fixture("int_bigint_ext.py"))
        .arg("-o")
        .arg(dir.join("int_bigint_ext"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let compiled = python()
        .arg("-c")
        .arg(EXT_PROGRAM)
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    let source_dir = ScratchDir::new("int_bigint_ext_source").expect("scratch");
    std::fs::copy(
        fixture("int_bigint_ext.py"),
        source_dir.join("int_bigint_ext.py"),
    )
    .expect("copy the source");
    let oracle = python()
        .arg("-c")
        .arg(EXT_PROGRAM)
        .current_dir(&*source_dir)
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert!(compiled.status.success(), "{}", rendered(&compiled));
    assert_eq!(stdout(&compiled), stdout(&oracle));
}

/// Builds `tests/fixtures/<module>.py` as an extension module in a fresh
/// scratch dir.
fn build_ext_fixture(tag: &str, module: &str) -> ScratchDir {
    let dir = ScratchDir::new(tag).expect("scratch");
    let build = pycc()
        .arg("build")
        .arg(fixture(&format!("{module}.py")))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    dir
}

fn run_in(dir: &Path, program: &str) -> Output {
    python()
        .arg("-c")
        .arg(program)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

/// A large `int` passed from Python into a position that needs a machine
/// word (an index, a slice bound, a repeat count, a container element, a
/// buffer index) raises a Python exception; it never aborts the host
/// interpreter. Index and repeat count raise CPython's own exceptions, a
/// container element holds it (v0.6), and a list slice bound (`IndexError`)
/// raises the recorded gap (`docs/DEVIATIONS.md`).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_large_int_in_a_machine_word_position_raises_instead_of_aborting_the_host() {
    let dir = build_ext_fixture("int_bigint_boundary", "int_bigint_boundary");
    let run = run_in(
        &dir,
        "\
import array, int_bigint_boundary as m
big = 2**100
cases = [
    ('at', lambda: m.at(big)),
    ('at_negative', lambda: m.at(-big)),
    ('push', lambda: m.push(big)),
    ('head', lambda: m.head(big)),
    ('rep', lambda: m.rep(big)),
    ('buf_at', lambda: m.buf_at(memoryview(array.array('d', [1.0])), big)),
]
for name, call in cases:
    try:
        call()
    except (IndexError, OverflowError) as e:
        print(name, type(e).__name__, e)
    else:
        print(name, 'no exception')
print(m.at(2), m.push(5), m.head(2), m.rep(2), m.divide(-big, 7) == -big // 7)
",
    );
    assert!(run.status.success(), "{}", rendered(&run));
    assert_eq!(
        stdout(&run),
        "at IndexError cannot fit 'int' into an index-sized integer\n\
         at_negative IndexError cannot fit 'int' into an index-sized integer\n\
         push no exception\n\
         head IndexError cannot fit 'int' into an index-sized integer\n\
         rep OverflowError cannot fit 'int' into an index-sized integer\n\
         buf_at IndexError cannot fit 'int' into an index-sized integer\n\
         3 2 2 abab True\n"
    );
}

/// An `ext` module's pending-exception check reads the calling thread's own
/// state: one Python thread raising inside the module never changes the
/// control flow of another thread's calls.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn ext_exception_state_is_per_thread() {
    let dir = build_ext_fixture("int_bigint_threads", "int_bigint_ext");
    let run = run_in(
        &dir,
        "\
import sys, threading, int_bigint_ext as m
sys.setswitchinterval(1e-6)
expected = m.fib(200)
errors = []
def raiser():
    for _ in range(20000):
        try:
            m.floor_pair(2**100, 0)
        except ZeroDivisionError:
            pass
        else:
            errors.append('no ZeroDivisionError')
def worker():
    for _ in range(20000):
        if m.fib(200) != expected or m.floor_pair(2**100, 7) != divmod(2**100, 7):
            errors.append('wrong result')
threads = [threading.Thread(target=raiser), threading.Thread(target=worker)]
for t in threads:
    t.start()
for t in threads:
    t.join()
print(errors[:3], len(errors))
",
    );
    assert!(run.status.success(), "{}", rendered(&run));
    assert_eq!(stdout(&run), "[] 0\n");
}
