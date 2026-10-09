// Issue #770 review (D-197, #763, Part 1 of #747): a value-less
// `Optional[T]` declaration (`x: int | None`, no initializer) followed by a
// plain reassignment (`x = 5`) lost its `Optional` annotation during MIR
// lowering. `pycc_mir::stmt::lower_stmt`'s `HirStmt::Assign` arm always
// bound the target's MIR scope type -- and, via
// `pycc_codegen::collect_stmt_bindings`'s own independent derivation from
// the first real `MirStmt::Assign`, the target's actual storage-slot type
// -- to the plain reassignment value's own bare `.ty()`, silently
// forgetting that the name had ever been declared `Optional[int]`. A later
// `x is None` then panicked in `pycc_codegen::emit_expr` because its
// non-`None` operand was not `Optional[_]`.
//
// The fix is two-part in `crates/pycc_mir/src/stmt.rs`:
//   1. a value-less `AnnAssign` under an `Optional[inner]` annotation now
//      records that declared type in MIR lowering's own `scopes` (unlike
//      every other value-less `AnnAssign`, which still binds nothing);
//   2. a plain `HirStmt::Assign` whose target is already scoped as
//      `Optional[inner]` now re-wraps its value via `MirExpr::OptionalWrap`
//      when the value isn't already that same `Optional` type.
//
// `crates/pycc_mir/src/tests/stmt.rs` covers the MIR shape directly; these
// tests cover the same fix end-to-end through the full compiler pipeline.

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
    let dir = ScratchDir::new(&format!("770_{tag}")).expect("failed to create scratch dir");
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

/// The exact #770 review repro: `x: int | None` (no initializer), then a
/// plain `x = 5`, then `print(x is None)` must print `False` -- before the
/// fix this panicked in codegen instead of running at all.
#[test]
fn a_plain_reassignment_after_a_value_less_optional_declaration_prints_false_for_is_none() {
    assert_builds_and_prints(
        "reassign_int",
        "x: int | None\nx = 5\nprint(x is None)\n",
        "False\n",
    );
}

/// The `None`-reassignment counterpart: `x = None` after the same
/// value-less declaration must print `True`.
#[test]
fn a_plain_none_reassignment_after_a_value_less_optional_declaration_prints_true_for_is_none() {
    assert_builds_and_prints(
        "reassign_none",
        "x: int | None\nx = None\nprint(x is None)\n",
        "True\n",
    );
}

/// Reading a value-less `Optional` declaration before any assignment is
/// still rejected by `pycc_types`' own definite-assignment checking --
/// confirming the MIR fix (which records the declared type in scope
/// purely for a *later* plain reassignment's own lookup) does not weaken
/// that separate, upstream guarantee. `pycc_types::check` runs before
/// `pycc_mir::build` in the compiler pipeline (see `src/main.rs`), so this
/// program never reaches MIR lowering at all.
#[test]
fn reading_a_value_less_optional_declaration_before_assignment_is_still_rejected() {
    let dir = ScratchDir::new("770_unbound").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "case.py", "x: int | None\nprint(x is None)\n");
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc check should still reject reading `x` before any assignment"
    );
}
