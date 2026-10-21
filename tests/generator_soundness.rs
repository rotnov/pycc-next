//! The generator protocol never reads a value as a type it does not have.
//!
//! * `e.value` of `except StopIteration as e` is typed from the generators
//!   the `try` body resumes itself, but the caught `StopIteration` may come
//!   from a generator some call in the body resumed: its value is tagged
//!   with its type at the `raise`, and one of another type raises
//!   `TypeError` instead of being reinterpreted (`docs/DEVIATIONS.md`).
//! * `gen.send(None)` is checked against the generator's send type like any
//!   other value: a `yield` in a generator taking `int` values is read as an
//!   `int`, so sending it `None` is a compile-time `T0021`.
//!
//! * A `None`-typed slot -- `got = yield v` in a generator whose send type
//!   is `None`, `x = None` -- is stored as the one-byte unit carrier, never
//!   as the two-byte `None` literal placeholder: that store overran the
//!   slot, which a debug build (where it is not optimized away) turned into
//!   a lost yield or a crash depending on the frame layout.
//!
//! What CPython runs the same way is compared with CPython byte for byte.

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

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

/// Builds `path` (in the release profile when `release`) into `dir` and
/// runs it.
fn build_and_run(dir: &ScratchDir, path: &Path, release: bool) -> Output {
    let out = dir.join("prog");
    let mut build = pycc();
    build.arg("build").arg(path).arg("-o").arg(&out);
    if release {
        build.arg("--release");
    }
    let build = build.output().expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        text(&pycc_scratch::lf(&build.stdout)),
        text(&pycc_scratch::lf(&build.stderr))
    );
    Command::new(&out).output().expect("the program runs")
}

/// `send(None)` to a generator taking `None` values, `next()` priming one
/// taking `int` values, the just-started generator's `TypeError`, and a
/// `StopIteration` a helper raises from a generator of the same return type.
#[test]
fn sends_and_helper_stop_iterations_match_cpython() {
    let name = "generator_send_none.py";
    for release in [false, true] {
        let dir = ScratchDir::new("generator_soundness").expect("scratch");
        let ours = build_and_run(&dir, &fixture(name), release);
        let theirs = python().arg(fixture(name)).output().expect("python3 runs");
        assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&theirs.stdout)), "stdout differs");
        assert_eq!(last_line(&pycc_scratch::lf(&ours.stderr)), last_line(&pycc_scratch::lf(&theirs.stderr)));
        assert_eq!(ours.status.code(), theirs.status.code());
    }
}

/// A function returning a generator it does not iterate itself (`return
/// lengths(rows)`) is not a generator: it ran as one, ending at once, so
/// the caller saw an empty sequence.
#[test]
fn a_function_returning_a_generator_is_not_one() {
    let name = "generator_returned.py";
    for release in [false, true] {
        let dir = ScratchDir::new("generator_returned").expect("scratch");
        let ours = build_and_run(&dir, &fixture(name), release);
        let theirs = python().arg(fixture(name)).output().expect("python3 runs");
        assert!(theirs.status.success(), "{}", text(&pycc_scratch::lf(&theirs.stderr)));
        assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&theirs.stdout)), "stdout differs");
        assert_eq!(ours.status.code(), theirs.status.code());
    }
}

/// `send(None)` to a generator whose send type excludes `None`.
#[test]
fn sending_none_to_a_generator_taking_ints_is_a_type_error() {
    let dir = ScratchDir::new("generator_soundness_check").expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(
        &path,
        "from typing import Generator\n\n\ndef g() -> Generator[int, int, None]:\n    x = yield 1\n    print(x + 1)\n    yield 2\n\n\nit = g()\nnext(it)\nit.send(None)\n",
    )
    .expect("write");
    let out = pycc()
        .arg("check")
        .arg(&path)
        .output()
        .expect("pycc should spawn");
    let output = format!("{}{}", text(&pycc_scratch::lf(&out.stdout)), text(&pycc_scratch::lf(&out.stderr)));
    assert!(!out.status.success(), "{output}");
    assert!(
        output.contains("error[T0021]: `send()` to a generator taking `int` values, got `None`"),
        "{output}"
    );
}

const GENERATORS: &str = "from typing import Generator\n\n\n\
def gi() -> Generator[int, None, int]:\n    yield 1\n    return 42\n\n\n\
def gs() -> Generator[int, None, str]:\n    yield 1\n    return \"hello\"\n\n\n\
def gn() -> Generator[int, None, None]:\n    yield 1\n\n\n";

/// A `StopIteration` whose value has another type than the one `e.value` is
/// read as -- it came from a generator a helper resumed -- raises
/// `TypeError` rather than reinterpreting the value's word. (CPython would
/// hand over the value itself: `docs/DEVIATIONS.md`.)
#[test]
fn a_stop_iteration_value_of_another_type_is_never_reinterpreted() {
    for (body, message) in [
        // `e.value` read as `int`; the value is a `str`.
        (
            "def drain(g: Generator[int, None, str]) -> None:\n    next(g)\n    next(g)\n\n\n\
def main() -> None:\n    a = gi()\n    b = gs()\n    try:\n        next(a)\n        drain(b)\n    except StopIteration as e:\n        print(e.value + 1)\n\n\nmain()\n",
            "TypeError: StopIteration.value here comes from a generator whose return type differs",
        ),
        // `e.value` read as `None`; the value is an `int`.
        (
            "def drain(g: Generator[int, None, int]) -> None:\n    next(g)\n    next(g)\n\n\n\
def main() -> None:\n    a = gn()\n    b = gi()\n    try:\n        next(a)\n        drain(b)\n    except StopIteration as e:\n        print(e.value)\n\n\nmain()\n",
            "TypeError: StopIteration.value here comes from a generator whose return type differs",
        ),
        // `e.value` read as `str`; the generator returned nothing.
        (
            "def drain(g: Generator[int, None, None]) -> None:\n    next(g)\n    next(g)\n\n\n\
def main() -> None:\n    a = gs()\n    b = gn()\n    try:\n        next(a)\n        drain(b)\n    except StopIteration as e:\n        print(e.value)\n\n\nmain()\n",
            "TypeError: StopIteration.value is None here",
        ),
    ] {
        for release in [false, true] {
            let dir = ScratchDir::new("generator_soundness_cross").expect("scratch");
            let path = dir.join("subject.py");
            std::fs::write(&path, format!("{GENERATORS}{body}")).expect("write");
            let ours = build_and_run(&dir, &path, release);
            assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), "", "{body}");
            assert!(
                last_line(&pycc_scratch::lf(&ours.stderr)).starts_with(message),
                "{body}: {}",
                text(&pycc_scratch::lf(&ours.stderr))
            );
            assert_eq!(ours.status.code(), Some(1), "{body}");
        }
    }
}

/// `None`-typed slots in generators and plain functions, in both profiles,
/// memcheck-clean when valgrind is installed.
#[test]
fn none_typed_slots_match_cpython_in_both_profiles() {
    let name = "generator_none_slots.py";
    let theirs = python().arg(fixture(name)).output().expect("python3 runs");
    assert!(theirs.status.success(), "{}", text(&pycc_scratch::lf(&theirs.stderr)));
    for release in [false, true] {
        let dir = ScratchDir::new("generator_soundness_none").expect("scratch");
        let ours = build_and_run(&dir, &fixture(name), release);
        assert_eq!(
            text(&pycc_scratch::lf(&ours.stdout)),
            text(&pycc_scratch::lf(&theirs.stdout)),
            "stdout differs (release: {release})"
        );
        assert!(ours.status.success(), "{}", text(&pycc_scratch::lf(&ours.stderr)));
        let Ok(memcheck) = Command::new("valgrind")
            .args(["-q", "--error-exitcode=99", "--leak-check=no"])
            .arg(dir.join("prog"))
            .output()
        else {
            eprintln!("valgrind not installed: memcheck skipped");
            continue;
        };
        assert!(
            memcheck.status.success(),
            "memcheck (release: {release}): {}",
            text(&pycc_scratch::lf(&memcheck.stderr))
        );
    }
}
