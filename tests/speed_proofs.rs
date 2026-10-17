//! Oracle and IR test for the v0.2 speed proofs (`tests/fixtures/speed_proofs.py`):
//! the compiled program prints exactly what CPython prints in both build
//! profiles, and the checks a proof removes are gone exactly where the proof
//! holds and kept where it does not.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn rendered(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout(output),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/speed_proofs.py")
}

/// Builds the fixture (dumping its LLVM IR before optimization) and returns
/// the compiled program's output and the IR.
fn build_and_run(dir: &ScratchDir, release: bool) -> (Output, String) {
    let binary = dir.join("prog");
    let ir = dir.join("prog.ll");
    let mut command = Command::new(env!("CARGO_BIN_EXE_pycc"));
    command
        .arg("build")
        .arg(fixture())
        .arg("-o")
        .arg(&binary)
        .env("PYCC_DUMP_LLVM", &ir);
    if release {
        command.arg("--release");
    }
    let build = command.output().expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let compiled = Command::new(&binary).output().expect("the program should spawn");
    let mut pre = ir.into_os_string();
    pre.push(".pre.ll");
    let ir = std::fs::read_to_string(&pre)
        .expect("PYCC_DUMP_LLVM writes the unoptimized IR")
        .replace("\r\n", "\n");
    (compiled, ir)
}

fn check(release: bool) {
    let dir = ScratchDir::new(&format!("speed_proofs_{release}")).expect("scratch");
    let (compiled, _) = build_and_run(&dir, release);
    let oracle = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg(fixture())
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert!(compiled.status.success(), "{}", rendered(&compiled));
    assert_eq!(stdout(&compiled), stdout(&oracle));
}

/// The body of the LLVM function `name` in `ir`.
fn function<'a>(ir: &'a str, name: &str) -> &'a str {
    let header = format!("@{name}(");
    let start = ir
        .match_indices("\ndefine ")
        .map(|(at, _)| at)
        .find(|&at| ir[at + 1..].lines().next().is_some_and(|line| line.contains(&header)))
        .unwrap_or_else(|| panic!("no function {name} in the IR"));
    let end = ir[start..].find("\n}\n").expect("a function ends") + start;
    &ir[start..end]
}

#[test]
fn speed_proofs_match_cpython_in_release() {
    check(true);
}

#[test]
fn speed_proofs_match_cpython_in_debug() {
    check(false);
}

#[test]
fn proven_checks_are_dropped_and_unproven_ones_kept() {
    let dir = ScratchDir::new("speed_proofs_ir").expect("scratch");
    let (_, ir) = build_and_run(&dir, false);
    let checked = "@pycc_rt_instance_get_slot_checked(";
    // `Point`'s constructor assigns both attributes before anything sees
    // the instance: no read tests the flag, and no store outside the
    // constructor sets it again.
    let points = function(&ir, "pyfn_points");
    assert!(!points.contains(checked), "{points}");
    assert!(!points.contains("slot_assigned"), "{points}");
    // `Escapes` hands `self` to `note` before assigning `second`.
    assert!(function(&ir, "pyfn_note").contains(checked));
    // `Derived` never assigns the `v` it inherits from `Base`, so neither
    // class proves `v`.
    assert!(function(&ir, "pyfn_show_v").contains(checked));
    // `Partial` may return before assigning `b`.
    assert!(function(&ir, "pyfn_classes").contains(checked));
    // `early` is defined before any statement runs; `after` is not.
    assert!(!function(&ir, "pyfn_calls").contains("@fnptr_early"));
    assert!(function(&ir, "pyfn_call_after").contains("@fnptr_after"));
    // `fib` calls only itself and cannot raise: no pending-exception test.
    assert!(!function(&ir, "pyfn_fib").contains("@pycc_rt_exc_pending"));
    assert!(function(&ir, "pyfn_classes").contains("@pycc_rt_exc_pending"));
    // A tree of `int` operators shares one fast path.
    assert_eq!(function(&ir, "pyfn_tri").matches("int_tree_fast:").count(), 1);
}

/// A negative finite base of `** 0.5` still raises from the cold path
/// (CPython would return a complex; pycc raises, see the runtime's
/// `float_pow`), after the loop's earlier iterations ran.
#[test]
fn a_negative_base_of_a_square_root_still_raises() {
    let dir = ScratchDir::new("speed_proofs_neg_sqrt").expect("scratch");
    let source = dir.join("neg.py");
    std::fs::write(
        &source,
        "def roots(xs: list[float]) -> None:\n    for x in xs:\n        print(x ** 0.5)\n\n\nroots([4.0, 0.0, -0.0, 2.0, -4.0, 9.0])\n",
    )
    .expect("write the program");
    let binary = dir.join("neg");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .arg("--release")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = Command::new(&binary).output().expect("the program should spawn");
    assert_eq!(run.status.code(), Some(1), "{}", rendered(&run));
    assert_eq!(stdout(&run), "2.0\n0.0\n0.0\n1.4142135623730951\n");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("RuntimeError"),
        "{}",
        rendered(&run)
    );
}
