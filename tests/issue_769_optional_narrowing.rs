// Issue #769 (Part 2 of #747), D-199: flow-sensitive `Optional[int]`
// narrowing on `is None` / `is not None`. `tests/fixtures/pep_0604_union.py`
// exercises the same feature through `tests/conformance.rs`'s CPython
// byte-for-byte oracle comparison, but that comparison is skipped whenever
// the pinned `python3.14` oracle is unavailable on `PATH` (see
// `conformance.rs`'s `oracle_python_bin`). This file gives the bigint-payload
// narrowed-read scenario an oracle-independent, self-contained regression
// test so it is exercised (and its codegen path covered) even in an
// environment without that pinned CPython build installed.
//
// Specifically: assigning a narrowed `Optional[int]` read into a second
// binding forces codegen's duplicate-reference retain path
// (`pycc_codegen::bigint_rc::retain_if_int_duplicate`) to run on a
// `MirExpr::OptionalUnwrap` source rather than a plain `int` source -- the
// scenario is only reachable when the unwrapped payload is a heap-allocated
// bigint (2**62 and above), since a smallint payload never reaches that
// function's `Scalar::Int` guard at all.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

/// Builds and runs `source`, asserting both steps succeed and stdout
/// matches `expected`.
fn assert_builds_and_prints(tag: &str, source: &str, expected: &str) {
    let dir = ScratchDir::new(&format!("769_{tag}")).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "case.py", source);
    let out = dir.join("case");
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "pycc build should succeed for `{tag}`; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&out).output().unwrap();
    assert!(
        run.status.success(),
        "the built binary for `{tag}` should run successfully; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        expected,
        "unexpected stdout for `{tag}`"
    );
}

/// Narrowing a bigint-valued `Optional[int]` and duplicating the narrowed
/// read into a second binding must retain the shared bigint reference: both
/// bindings have to remain independently readable (and, at process exit,
/// independently releasable) after the narrowed branch closes.
/// 4611686018427387904 is 2**62, the same promoted-to-bigint boundary value
/// used by `tests/fixtures/bigint_range.py` and `tests/issue_147_bigint_range.rs`.
#[test]
fn narrowing_a_present_bigint_optional_and_duplicating_the_read_retains_both_bindings() {
    assert_builds_and_prints(
        "bigint_duplicate",
        "big: int | None = 4611686018427387904\n\
         if big is not None:\n    \
             duplicated = big\n    \
             print(big + 1)\n    \
             print(duplicated)\n",
        "4611686018427387905\n4611686018427387904\n",
    );
}

/// The mirror smallint case: narrowing and duplicating a smallint-valued
/// `Optional[int]` payload must also work, confirming the duplicate-retain
/// codegen path's `Scalar::Int` guard correctly no-ops for an inline value
/// rather than misclassifying it as a heap reference.
#[test]
fn narrowing_a_present_smallint_optional_and_duplicating_the_read_prints_both_bindings() {
    assert_builds_and_prints(
        "smallint_duplicate",
        "small: int | None = 41\n\
         if small is not None:\n    \
             duplicated = small\n    \
             print(small + 1)\n    \
             print(duplicated)\n",
        "42\n41\n",
    );
}

/// An absent (`None`) `Optional[int]` never enters the narrowed branch at
/// all -- the duplicate-retain path must not run, and the program takes the
/// `is None` branch instead.
#[test]
fn narrowing_an_absent_optional_never_enters_the_narrowed_branch() {
    assert_builds_and_prints(
        "absent",
        "absent: int | None = None\n\
         if absent is not None:\n    \
             duplicated = absent\n    \
             print(duplicated)\n\
         else:\n    \
             print(\"absent\")\n",
        "absent\n",
    );
}
