// Issue #167: D-075 None-carrier user-function-ABI regression tests.
//
// Extracted from `tests/slice1_codegen_depth.rs` (AGENTS.md "Keep source
// files decomposable": that file and `crates/pycc_codegen/src/tests.rs`
// are both past the ~1,000-line threshold, and this PR's own work touches
// the `None`-carrier ABI cluster inside the former).
//
// D-075 promises that a `None` value crossing the user-function ABI is the
// canonical LLVM `i8 0` unit carrier. These two tests observe that carrier
// from opposite ends: the first only proves the static `Ty::None` type tag
// prints "None" (true for *any* carrier bit pattern), the second actually
// branches on the parameter's truthiness, which pycc lowers through the
// same `truthy` path as any other typed value, so a flipped carrier bit
// changes which branch prints.

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

fn build_and_run(label: &str, source: &str) -> std::process::Output {
    let dir = ScratchDir::new(&format!("issue167_{label}")).expect("failed to create scratch dir");
    let src = write_fixture(&dir, &format!("{label}.py"), source);
    let out = dir.join(label);
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "`pycc build` failed for {label}");
    Command::new(&out).output().unwrap()
}

#[test]
fn none_typed_parameters_cross_the_user_function_abi() {
    let source = "\
def source() -> None:
    return

def sink(value: None) -> None:
    print(value)
    return value

sink(source())
";
    let output = build_and_run("none_parameter_abi", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"None\n");
}

/// #167: D-075 promises the canonical `None` unit carrier crossing the user
/// function ABI is LLVM `i8 0`, but `none_typed_parameters_cross_the_user_function_abi`
/// only prints the statically-known `Ty::None`, which renders "None" for
/// *any* carrier bit pattern and so cannot observe the carrier value at all.
/// This branches on the parameter's truthiness instead, which pycc lowers
/// through the same `truthy` path as any other typed value -- a `1` carrier
/// would flip the printed branch, so this fails under the mutation that
/// motivated the issue (`Scalar::Bool(const_int(0, ...))` ->
/// `Scalar::Bool(const_int(1, ...))` in the `None` call-result carrier).
#[test]
fn a_none_call_result_crossing_the_abi_carries_a_falsy_unit_value() {
    let source = "\
def source() -> None:
    return

def sink(value: None) -> None:
    if value:
        print(1)
    else:
        print(0)

sink(source())
";
    let output = build_and_run("none_carrier_truthiness", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"0\n");
}
