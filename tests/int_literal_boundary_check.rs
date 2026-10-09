// Issue #618: an out-of-range `int` *literal* in a runtime int-boundary
// position (D-141) is now rejected at compile time (T0051) instead of
// reaching `pycc_rt_int_untag_checked` and aborting at run time -- the
// consequence D-178 (PR #617, closing #148) knowingly accepted when it made
// an out-of-range literal materialize as a heap bigint everywhere else.
//
// `crates/pycc_hir/src/expr.rs`'s own `mod tests` already exercises every
// one of the 13 named positions directly against `lower_checked`, which is
// what D-014's coverage gate needs. This file instead proves the two
// end-to-end behaviors real Python source must keep, front to back through
// the real `pycc` CLI (mirroring `container_methods1_codegen_depth.rs`'s own
// `build_and_run` convention):
//
// 1. A literal in a boundary position now fails `pycc check`/`pycc build`
//    with a non-zero exit instead of producing a binary that traps at run
//    time (criterion 1).
// 2. A bigint value reaching the same position through *arithmetic* (not a
//    literal) is completely unaffected: it still compiles, and the
//    resulting binary still aborts at run time exactly as it did before
//    this issue (criterion 2) -- this issue narrows only the literal case.

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

#[test]
fn a_literal_in_a_boundary_position_fails_check_instead_of_building() {
    let dir =
        ScratchDir::new("int_boundary_literal_check_fails").expect("failed to create scratch dir");
    let source = "\
xs = [1]
xs.append(4611686018427387904)
";
    let src = write_fixture(&dir, "literal_boundary.py", source);

    let check_output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !check_output.status.success(),
        "pycc check must reject an out-of-range int literal in a boundary position"
    );
    let stderr_and_stdout = format!(
        "{}{}",
        String::from_utf8_lossy(&check_output.stdout),
        String::from_utf8_lossy(&check_output.stderr)
    );
    assert!(
        stderr_and_stdout.contains("T0051"),
        "expected a T0051 diagnostic, got: {stderr_and_stdout}"
    );

    let out = dir.join("literal_boundary");
    let build_status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        !build_status.success(),
        "pycc build must also reject the same out-of-range int literal, not just pycc check"
    );
}

#[test]
fn a_bigint_reaching_a_boundary_position_through_arithmetic_still_builds_and_traps_at_runtime() {
    let dir = ScratchDir::new("int_boundary_arithmetic_still_traps")
        .expect("failed to create scratch dir");
    // Neither operand is itself out of range (4611686018427387903 is the
    // largest tagged smallint -- see `crates/pycc_rt/src/lib.rs`'s own
    // `fits_smallint` tests), but their sum, `4611686018427387904`, is not:
    // `int_add` materializes it as a heap bigint (D-178), and
    // `xs.append(...)`'s own boundary check on that heap value keeps
    // aborting at run time, unaffected by this issue's compile-time literal
    // check (criterion 2).
    let source = "\
xs = [1]
a = 4611686018427387903
b = 1
xs.append(a + b)
print(len(xs))
";
    let src = write_fixture(&dir, "arithmetic_boundary.py", source);

    let check_status = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        check_status.success(),
        "pycc check must accept a bigint reaching a boundary position through arithmetic"
    );

    let out = dir.join("arithmetic_boundary");
    let build_status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        build_status.success(),
        "pycc build must still produce a binary for the arithmetic case"
    );

    let run_output = Command::new(&out).output().unwrap();
    assert!(
        !run_output.status.success(),
        "the built binary must still abort at run time when the boundary actually receives a \
         bigint, exactly as before this issue -- only the literal case is now caught earlier"
    );
}
