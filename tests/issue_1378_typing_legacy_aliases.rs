// Issue #1378 (Part 6 of #882): `from typing import Dict, Any, Generic, List`
// resolves, and the pre-PEP 585 container aliases `Dict`, `List`, `Set`,
// `FrozenSet` and `Tuple` lower to exactly the builtin container they name.
//
// Exercised through the public CLI: `pycc check` accepts the import line,
// and `pycc build` produces a binary whose output is identical to the same
// program spelled with the builtin containers -- the observable definition
// of "an alias, not a new type".

use pycc_scratch::ScratchDir;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

/// Every legacy alias in a parameter, return, local and module-level
/// annotated position.
const LEGACY: &str = "\
from typing import Dict, Any, Generic, List, Set, FrozenSet, Tuple

def total(d: Dict[str, int], xs: List[int]) -> Tuple[int, int]:
    seen: Set[int] = {xs[0], xs[1], xs[2]}
    frozen: FrozenSet[int] = frozenset({xs[0], xs[1]})
    return (d[\"a\"] + len(seen), len(frozen))

pair: Tuple[int, int] = total({\"a\": 40}, [1, 2, 2])
print(pair[0])
print(pair[1])
";

/// The same program spelled with the builtin containers.
const BUILTIN: &str = "\
def total(d: dict[str, int], xs: list[int]) -> tuple[int, int]:
    seen: set[int] = {xs[0], xs[1], xs[2]}
    frozen: frozenset[int] = frozenset({xs[0], xs[1]})
    return (d[\"a\"] + len(seen), len(frozen))

pair: tuple[int, int] = total({\"a\": 40}, [1, 2, 2])
print(pair[0])
print(pair[1])
";

fn run_pycc(args: &[&str]) -> std::process::Output {
    Command::new(pycc_bin()).args(args).output().unwrap()
}

fn build_and_run(dir: &std::path::Path, stem: &str, source: &str) -> String {
    let src = dir.join(format!("{stem}.py"));
    std::fs::write(&src, source).unwrap();
    let exe = dir.join(stem);
    let build = run_pycc(&["build", src.to_str().unwrap(), "-o", exe.to_str().unwrap()]);
    assert!(
        build.status.success(),
        "pycc build should succeed for `{stem}`; stdout: {} stderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(
        run.status.success(),
        "`{stem}` should exit successfully; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n")
}

/// #1378: the import line checks clean. Before this change it failed with
/// `C0002` ("module `typing` has no importable symbol named `Dict`").
#[test]
fn the_legacy_typing_import_line_checks_successfully() {
    let dir = ScratchDir::new("1378_check").expect("failed to create scratch dir");
    let src = dir.join("legacy.py");
    std::fs::write(&src, LEGACY).unwrap();
    let output = run_pycc(&["check", src.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "pycc check should accept the legacy typing aliases; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// #1378: the legacy spellings compile to exactly what the builtin ones do.
#[test]
fn the_legacy_aliases_build_to_the_builtin_containers_output() {
    let dir = ScratchDir::new("1378_build").expect("failed to create scratch dir");
    let legacy = build_and_run(&dir, "legacy", LEGACY);
    let builtin = build_and_run(&dir, "builtin", BUILTIN);
    assert_eq!(legacy, builtin);
    assert_eq!(legacy, "42\n2\n");
}
