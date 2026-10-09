use pycc_scratch::ScratchDir;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

/// This PR's plan (Task 3 Step 6a, moved here from Task 2's original Step 8
/// once `--release` itself existed to combine with) asks for a narrow, real
/// consumption point: `pycc build`, given no explicit `--release` flag,
/// should still build in the release profile when a `pycc.toml` next to
/// the source file names `[build] opt = "release"`.
///
/// This test proves only that the real CLI *reaches `compile_to_object`
/// without erroring* and produces a correctly running binary, given a
/// relative source path (`main.py`, `current_dir`-ed into the project
/// directory) and no `--release` flag, with a neighboring `pycc.toml`
/// naming `opt = "release"` present. It does *not* prove that LLVM's O3
/// pipeline actually ran for this build, nor that the release bool
/// actually reached `compile_to_object` as `true` -- this fixture's output
/// would be identical either way, since `--release` changes optimization,
/// not observable program behavior. Two other tests carry those specific
/// claims: `pycc_codegen`'s own `release_mode_actually_runs_llvm_
/// optimization_passes` unit test proves LLVM's `"default<O3>"` pipeline
/// ran by observing that exact pipeline and its removal of an unused runtime
/// declaration immediately before object emission, while retaining a used
/// declaration (an earlier version of *this* test tried to prove that here
/// too, comparing the final *linked* binary's size -- see
/// `docs/AGENT_RETROSPECTIVE.md`'s 2026-07-28 entry for why that proxy has no
/// signal at this level: a statically-linked runtime and Mach-O segment-
/// alignment padding absorb the relevant code-size delta).
/// `src/main.rs`'s own `release_flag_tests` prove `resolve_release_flag`
/// correctly derives `true` from exactly this `pycc.toml` shape (and
/// falls back to `false` for every other input, including a malformed
/// neighboring file), and `try_build_release_isolation_tests` proves
/// `try_build` itself never consults a neighboring `pycc.toml` at all --
/// only `main()`'s `Command::Build` arm does, before ever calling
/// `try_build`, so `pycc run` (which has no `--release` flag) stays
/// unaffected by a neighboring `pycc.toml`'s release default. This test's
/// job is narrower than any of those: proving the relative-path / cwd-
/// relative-read route through the real CLI doesn't panic or exit non-zero.
#[test]
fn build_with_a_relative_path_and_neighboring_pycc_toml_still_produces_a_running_binary() {
    let dir = ScratchDir::new("toml_release_default").expect("failed to create scratch dir");
    std::fs::write(
        dir.join("main.py"),
        "def main() -> None:\n    print(42)\n\nmain()\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("pycc.toml"),
        "[project]\nname = \"t\"\nentry = \"main.py\"\npython = \"3.14\"\n\n\
         [build]\nopt = \"release\"\n",
    )
    .unwrap();
    let out = dir.join("out");

    // Relative path (`main.py`), not absolute: `main()`'s `Command::Build`
    // arm passes this raw CLI argument straight to `resolve_release_flag`,
    // so this is what actually exercises its `source_path.parent() ==
    // Some("")` branch and the resulting cwd-relative `pycc.toml` read --
    // the genuinely non-obvious part of this wiring, not reproducible by
    // passing an absolute path.
    let status = Command::new(pycc_bin())
        .args(["build", "main.py", "-o", out.to_str().unwrap()])
        .current_dir(&*dir)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build failed with a neighboring release-profile pycc.toml and no \
         explicit --release flag"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"42\n");
}
