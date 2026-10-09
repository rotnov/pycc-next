// Issue #762: `typing.Final` and `typing.Annotated` are already fully
// supported as bare-name annotation subscripts (PEP 591/593,
// `pycc_hir::func::annotation_to_ty`), but were not registered in
// `pycc_std`'s symbol registry -- so the idiomatic `from typing import
// Final, Annotated` was unconditionally rejected with `C0002` even though
// the underlying feature worked perfectly once the import was omitted.
//
// These tests exercise the fix end-to-end: compiling a .py source that
// *does* import `Final`/`Annotated` from `typing`, using both a
// module-level `Final[int]` annotated assignment and a function parameter
// annotated `Annotated[int, ...]`, and asserting `pycc check`/`pycc build`
// both succeed.

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

const SOURCE: &str = "\
from typing import Final, Annotated

MAX_CONNECTIONS: Final[int] = 100

def scale(x: Annotated[int, \"meters\"]) -> int:
    return x * 2

print(MAX_CONNECTIONS)
print(scale(21))
";

/// #762: `pycc check` accepts `from typing import Final, Annotated` used
/// with both a module-level `Final[int]` annotated assignment and a
/// function parameter annotated `Annotated[int, ...]`.
#[test]
fn typing_final_and_annotated_import_check_succeeds() {
    let dir = ScratchDir::new("762_check").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "final_annotated.py", SOURCE);
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "pycc check should succeed for `from typing import Final, Annotated`; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// #762: `pycc build` accepts the same source and produces a binary that
/// runs and prints the expected output -- the import no longer changes
/// runtime behavior versus omitting it (the two PEP conformance fixtures
/// already cover that behavior without the import).
#[test]
fn typing_final_and_annotated_import_build_and_run_succeeds() {
    let dir = ScratchDir::new("762_build").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "final_annotated.py", SOURCE);
    let out = dir.join("final_annotated");
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "pycc build should succeed for `from typing import Final, Annotated`; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&out).output().unwrap();
    assert!(
        run.status.success(),
        "the built binary should run successfully"
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "100\n42\n",
        "MAX_CONNECTIONS then scale(21) should print 100 then 42"
    );
}

/// #762: importing an unregistered `typing` name still fails with `C0002`
/// -- the fix must not accidentally widen the registry beyond `Final`/
/// `Annotated`.
#[test]
fn typing_unregistered_symbol_import_still_rejected() {
    let dir = ScratchDir::new("762_reject").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "unregistered.py", "from typing import Callable\n");
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc check should still reject `from typing import Callable`"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("C0002"),
        "expected a C0002 diagnostic, got: {combined}"
    );
}
