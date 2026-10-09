//! `try_build`'s `--ext` wiring tests, moved out of `src/build_pipeline.rs`
//! when #1366 added one (AGENTS.md "Keep source files decomposable"); the
//! moved tests changed only for #1366's new `foreign_relative_imports`
//! argument.

use super::*;
use ext_build::{ExtProbe, ExtToolchain};
use pycc_scratch::ScratchDir;

/// A toolchain whose headers are `dir` itself, holding a `Python.h` that
/// is not one: a single `#error` line. Every step up to and including
/// the compiler spawn then runs for real, and the build fails
/// deterministically inside `cc` on any host, with no CPython installed
/// and nothing `#[ignore]`d. That is the only way the ext branch's
/// effectful tail earns coverage: `.github/workflows/ci.yml`'s coverage
/// job runs `llvm-cov` without `--include-ignored`.
///
/// The stub file is what keeps that reachable. The probe now rejects a
/// header directory with no `Python.h` as an environment failure, so an
/// empty directory would stop the build two steps earlier and leave the
/// whole tail uncovered -- the failure has to come from the *contents*
/// of a header, which is a compile error, not from its absence, which is
/// a broken build environment.
fn header_less_toolchain(dir: &Path) -> ExtToolchain {
    std::fs::write(
        dir.join("Python.h"),
        "#error pycc test fixture: not a real Python.h\n",
    )
    .expect("write the stub header");
    ExtToolchain::with_probe(
        "pycc-unused-interpreter",
        ExtProbe {
            version: ext_build::MIN_PYTHON,
            include: dir.to_path_buf(),
            libs: dir.join("libs"),
        },
    )
}

fn write_source(dir: &Path, body: &str) -> std::path::PathBuf {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write source");
    src
}

fn typed(src: &Path) -> pycc_hir::HirModule {
    resolve_frontend(src, Some(frontend::NATIVE_MODULE_NAME))
        .unwrap_or_else(|_| panic!("the fixture must type-check"))
}

#[test]
fn a_planned_ext_build_writes_both_c_files_beside_the_object() {
    let dir = ScratchDir::new("ext_plan").expect("scratch");
    let src = write_source(&dir, "def square(x: int) -> int:\n    return x * x\n");
    let obj = dir.join("main.o");
    let plan = plan_ext(
        &src,
        &dir.join("fastmath"),
        None,
        &typed(&src),
        &header_less_toolchain(&dir),
        &obj,
    )
    .expect("an int-only program plans cleanly");

    let inc = std::fs::read_to_string(dir.join(ext_build::EXPORTS_INC_NAME))
        .expect("the generated companion is written next to the object");
    assert!(
        inc.contains("#define PYCC_EXT_MODULE_NAME fastmath\n"),
        "{inc}"
    );
    assert!(inc.contains("fnptr_square"), "{inc}");
    let shim = std::fs::read_to_string(dir.join(ext_build::SHIM_C_NAME))
        .expect("the fixed shim is written next to the object");
    assert_eq!(shim, ext_build::SHIM_C);

    // The artifact is the resolved output, not `OUT` as given: comparing
    // `PathBuf`s built with `Path::join`, never rendered strings. This
    // call passes no target, so the suffix follows the *build host* --
    // two legal values, enumerated rather than branched on, so the
    // assertion holds on every Tier-1 host without a `cfg` arm that only
    // one of them ever executes.
    assert!(
        [dir.join("fastmath.abi3.so"), dir.join("fastmath.pyd")].contains(&plan.artifact),
        "{:?}",
        plan.artifact
    );
    assert!(plan.compile_args.contains(&std::ffi::OsString::from("-I")));
    assert!(!plan.link_args.is_empty());
}

#[test]
fn a_windows_target_plans_a_pyd_from_this_host() {
    let dir = ScratchDir::new("ext_plan_win").expect("scratch");
    let src = write_source(&dir, "def f() -> int:\n    return 1\n");
    let plan = plan_ext(
        &src,
        &dir.join("m"),
        Some("x86_64-pc-windows-msvc"),
        &typed(&src),
        &header_less_toolchain(&dir),
        &dir.join("main.o"),
    )
    .expect("a cross-target plan needs nothing from this host");
    assert_eq!(plan.artifact, dir.join("m.pyd"));
    assert!(
        plan.link_args
            .contains(&std::ffi::OsString::from("-lpython3"))
    );
}

#[test]
fn an_output_path_the_ext_contract_rejects_fails_before_the_export_scan() {
    let dir = ScratchDir::new("ext_plan_out").expect("scratch");
    let src = write_source(&dir, "def f() -> int:\n    return 1\n");
    let code = plan_ext(
        &src,
        Path::new("/"),
        None,
        &typed(&src),
        &header_less_toolchain(&dir),
        &dir.join("main.o"),
    )
    .expect_err("`/` names no module");
    assert_eq!(code, ExitCode::from(2));
}

#[test]
fn a_public_function_the_boundary_cannot_carry_fails_the_build_with_a_diagnostic() {
    let dir = ScratchDir::new("ext_plan_gap").expect("scratch");
    // `list[int]`, not `str`: #1049 made `str` carriable in both
    // positions, so the old fixture no longer reaches a gap.
    let src = write_source(&dir, "def greet(x: list[int]) -> int:\n    return 1\n");
    let code = plan_ext(
        &src,
        &dir.join("m"),
        None,
        &typed(&src),
        &header_less_toolchain(&dir),
        &dir.join("main.o"),
    )
    .expect_err("list is not bridged");
    assert_eq!(code, ExitCode::from(1));
}

#[test]
fn an_unusable_cpython_toolchain_fails_before_codegen() {
    let dir = ScratchDir::new("ext_plan_probe").expect("scratch");
    let src = write_source(&dir, "def f() -> int:\n    return 1\n");
    let toolchain = ExtToolchain::with_probe(
        "pycc-unused-interpreter",
        ExtProbe {
            version: ext_build::MIN_PYTHON,
            include: dir.join("no-such-include"),
            libs: dir.join("libs"),
        },
    );
    let code = plan_ext(
        &src,
        &dir.join("m"),
        None,
        &typed(&src),
        &toolchain,
        &dir.join("main.o"),
    )
    .expect_err("a missing header directory is an environment failure");
    assert_eq!(code, ExitCode::from(2));
}

#[test]
fn a_scratch_directory_that_cannot_be_written_is_an_environment_failure() {
    let dir = ScratchDir::new("ext_plan_write").expect("scratch");
    let src = write_source(&dir, "def f() -> int:\n    return 1\n");
    // An object path inside a directory that does not exist: the C
    // files land beside it, so writing them is what fails.
    let code = plan_ext(
        &src,
        &dir.join("m"),
        None,
        &typed(&src),
        &header_less_toolchain(&dir),
        &dir.join("no-such-dir").join("main.o"),
    )
    .expect_err("an unwritable scratch is an environment failure");
    assert_eq!(code, ExitCode::from(2));
}

/// A rejected `-o` is reported as the output-contract failure it is,
/// even when the source reads `__name__` (#1156).
///
/// The regression this pins: while the resolve's error was dropped to a
/// `None` module name, `dunder_name::seed_item` withheld the seed, the
/// type checker then rejected the program with `T0021` (exit 1), and
/// that unrelated diagnostic reached the user instead of the `-o` one.
/// Exit 2 is `resolve_ext_output`'s own code, so it distinguishes the
/// two outcomes without matching on rendered message text.
#[test]
fn a_rejected_ext_output_path_is_reported_even_when_the_source_reads_dunder_name() {
    let dir = ScratchDir::new("ext_out_reject").expect("scratch");
    let src = write_source(&dir, "def f() -> str:\n    return __name__\n");
    let code = try_build(
        &src,
        // `/` has no file-name component, so no module name can be
        // derived from it (`ext_output::ExtOutputError::NoFileName`).
        Path::new("/"),
        None,
        false,
        &dir.join("main.o"),
        Some(&header_less_toolchain(&dir)),
        &no_python(),
        InteropCli::default(),
        false,
    )
    .expect_err("`/` names no module");
    assert_eq!(code, ExitCode::from(2));
}

/// The whole `--ext` tail, end to end: output resolution, export scan,
/// toolchain probe, both C writes, codegen through
/// `CompileOptions { ext: true, .. }`, the runtime-library lookup, the
/// platform link argv, and the shared spawn. It fails inside `cc`,
/// because the include directory this test supplies holds no `Python.h`
/// -- which is exactly the point: every one of those steps ran.
#[test]
fn an_ext_build_runs_the_whole_tail_and_fails_in_the_compiler_without_python_headers() {
    let dir = ScratchDir::new("ext_try_build").expect("scratch");
    let src = write_source(&dir, "def square(x: int) -> int:\n    return x * x\n");
    let obj = dir.join("main.o");
    let code = try_build(
        &src,
        &dir.join("fastmath"),
        None,
        false,
        &obj,
        Some(&header_less_toolchain(&dir)),
        &no_python(),
        InteropCli::default(),
        false,
    )
    .expect_err("no Python.h means the compiler rejects the shim");
    assert_eq!(code, ExitCode::from(1));
    // Codegen really ran under `ext: true`, and really emitted the
    // module-body symbol the shim calls instead of `main`.
    let object = std::fs::read(&obj).expect("the object was emitted before the link");
    assert!(
        object
            .windows(pycc_codegen::EXT_MODULE_EXEC_SYMBOL.len())
            .any(|window| window == pycc_codegen::EXT_MODULE_EXEC_SYMBOL.as_bytes()),
        "the ext object must export the module-body symbol"
    );
}

/// #1366: with `foreign_relative_imports`, the ext arm resolves the
/// entry's relative from-import as a foreign import instead of refusing it
/// with a `T0021` (the module is outside any package and has no `sib.py`),
/// and codegen emits the relative import call. Both the frontend refusal
/// and the header-less compiler exit 1, so the object file -- written only
/// after the frontend accepted the module -- is what tells them apart.
#[test]
fn an_ext_build_with_foreign_relative_imports_emits_the_relative_import_call() {
    let dir = ScratchDir::new("1366_ext_try_build").expect("scratch");
    let src = write_source(
        &dir,
        "from .sib import x\n\n\ndef square(v: int) -> int:\n    return v * v\n",
    );
    let obj = dir.join("main.o");
    let code = try_build(
        &src,
        &dir.join("fastmath"),
        None,
        false,
        &obj,
        Some(&header_less_toolchain(&dir)),
        &no_python(),
        InteropCli::default(),
        true,
    )
    .expect_err("no Python.h means the compiler rejects the shim");
    assert_eq!(code, ExitCode::from(1));
    let object = std::fs::read(&obj).expect("the frontend accepted the relative import");
    let symbol = pycc_codegen::EXT_OBJ_IMPORT_FROM_SYMBOL.as_bytes();
    assert!(
        object.windows(symbol.len()).any(|window| window == symbol),
        "the ext object must call the from-import entry point"
    );
}
