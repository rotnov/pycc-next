use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Declares the harness's cohort submodules and records their paths so
/// `every_harness_module_on_disk_is_declared` can bind the on-disk set to the
/// compiled set. An integration-test crate root does not resolve a bare
/// `mod foo;` to `tests/conformance/foo.rs`, hence `#[path]`. New fixtures'
/// tests belong in the cohort file that owns their semantics, not here.
macro_rules! harness_modules {
    ($($name:ident => $path:literal),* $(,)?) => {
        $(#[path = $path] mod $name;)*
        const HARNESS_MODULE_FILES: &[&str] = &[$($path),*];
    };
}

harness_modules! {
    classes => "conformance/classes.rs",
    exceptions => "conformance/exceptions.rs",
    imports => "conformance/imports.rs",
    numeric => "conformance/numeric.rs",
}

/// The `conformance/<name>.rs` paths of every regular `*.rs` file directly in
/// `dir`, sorted. The `is_file` guard is the same one
/// `tests/harness_support/conformance_sources.rs` and
/// `scripts/check_conformance_breadth.py` apply, so this control's on-disk
/// set is exactly the set those readers concatenate (a *directory* named
/// `x.rs` is read by neither and must not be demanded here).
fn harness_modules_on_disk(dir: &Path) -> Vec<String> {
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "rs"))
        .map(|path| {
            format!(
                "conformance/{}",
                path.file_name().unwrap().to_string_lossy()
            )
        })
        .collect();
    on_disk.sort();
    on_disk
}

/// A cohort file that exists on disk but is not declared above would still be
/// read by the three harness text-readers (so its fixtures would count as
/// registered) while never being compiled or run. Runs by default, no oracle.
#[test]
fn every_harness_module_on_disk_is_declared() {
    let on_disk =
        harness_modules_on_disk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/conformance"));
    let mut declared: Vec<String> = HARNESS_MODULE_FILES.iter().map(|s| s.to_string()).collect();
    declared.sort();
    assert_eq!(
        on_disk, declared,
        "every tests/conformance/*.rs must be declared in harness_modules!"
    );
}

#[test]
fn harness_modules_on_disk_ignores_directories_and_non_rs_files() {
    let scratch = ScratchDir::new("harness_modules_on_disk").expect("scratch dir");
    let dir = scratch.join("conformance");
    std::fs::create_dir_all(dir.join("fake.rs")).expect("a directory named *.rs");
    std::fs::create_dir_all(dir.join("py30")).expect("nested dir");
    std::fs::write(dir.join("b.rs"), "").expect("b.rs");
    std::fs::write(dir.join("a.rs"), "").expect("a.rs");
    std::fs::write(dir.join("fixture.py"), "").expect("fixture.py");
    assert_eq!(
        harness_modules_on_disk(&dir),
        vec![
            "conformance/a.rs".to_string(),
            "conformance/b.rs".to_string()
        ]
    );
}

fn pycc_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

/// The pinned CPython 3.14.7 oracle (D-001's "python3.14" pin, advanced from
/// 3.14.6 in the 2026-08-15 oracle refresh). A missing or wrong-version oracle is a
/// clean, actionable panic, not a silently-skipped or falsely-passing check.
///
/// Windows needs its own file name here: `std::process::Command::new` on
/// Windows resolves a bare program name by searching `PATH` for that exact
/// file name and does not append `.exe` when the name already contains a
/// `.` (the version dot in "python3.14" reads as an extension to that
/// resolver). A CI-side `python3.14.exe` alias on `PATH` is therefore
/// invisible to this lookup even though shells like bash find it fine
/// (bash's own exec resolution does try appending `.exe`). Passing the
/// extension explicitly on Windows sidesteps the mismatch (D-080 addendum).
///
/// Testable core of `oracle_python_bin`: takes the "is this a Windows
/// target" check as a parameter instead of calling `cfg!(windows)` directly,
/// so one test can assert both filenames regardless of the host OS actually
/// running the test (matching `pycc_rt_lib_filename` in
/// `pycc_codegen::artifact_layout`,
/// which parameterizes an analogous host-vs-target naming choice the same
/// way after an earlier host-`cfg!`-keyed version was caught in review).
fn oracle_binary_name(is_windows: bool) -> &'static str {
    if is_windows {
        "python3.14.exe"
    } else {
        "python3.14"
    }
}

fn oracle_python_bin() -> PathBuf {
    let bin = PathBuf::from(oracle_binary_name(cfg!(windows)));
    let output = Command::new(&bin)
        .arg("--version")
        .output()
        .unwrap_or_else(|e| panic!("conformance oracle `python3.14` not found on PATH: {e}"));
    let version = String::from_utf8_lossy(&output.stdout);
    assert!(
        version.trim() == "Python 3.14.7",
        "conformance oracle must be exactly Python 3.14.7, found {version:?}"
    );
    bin
}

/// CPython's own stdio layer translates `\n` to `\r\n` on Windows even when
/// stdout is piped/redirected to a child-process capture like `Command::output()`
/// uses here (`sys.stdout`'s `TextIOWrapper` opens with `newline=None` there;
/// see CPython's bpo-11990 and bpo-13119, both intentional, stable behavior
/// since 3.2.4/3.3). `pycc_rt`'s `println!`/`print!`-based `print()` never
/// performs any such translation on any target. That translation is a
/// platform-specific quirk of CPython's own C runtime, not part of the
/// Python language's `print()` semantics, so it is stripped out of the
/// oracle's output before comparing -- this keeps the assertion checking
/// actual program-output content, not which OS's libc happened to run the
/// oracle (D-082).
fn strip_windows_newline_translation(bytes: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut iter = bytes.into_iter().peekable();
    while let Some(byte) = iter.next() {
        if byte == b'\r' && iter.peek() == Some(&b'\n') {
            continue;
        }
        out.push(byte);
    }
    out
}

/// Builds `py_path` with `pycc build --debug` (the default profile), runs
/// the resulting binary, separately runs the pinned CPython oracle on the
/// identical source, and returns both stdouts for the caller to diff.
fn run_conformance_fixture(label: &str, py_path: &Path) -> (Vec<u8>, Vec<u8>) {
    run_conformance_fixture_with_profile(label, py_path, false)
}

/// Same as `run_conformance_fixture`, but lets the caller choose the build
/// profile. New PEP fixtures (PR-9 on) must be proven in both `--debug` and
/// `--release` before their `docs/PYTHON_STANDARDS.md` row can flip to ✅ --
/// `docs/TESTING.md`'s "both profiles" rule stopped having a v0.1-only
/// exception once `--release` shipped in PR-8. `fib`/`mandelbrot` predate
/// that rule (neither is a PEP-matrix row) and stay on the plain,
/// `--debug`-only helper above rather than being retrofitted here.
fn run_conformance_fixture_with_profile(
    label: &str,
    py_path: &Path,
    release: bool,
) -> (Vec<u8>, Vec<u8>) {
    let profile = if release { "release" } else { "debug" };
    let dir = ScratchDir::new(&format!("conformance_{label}_{profile}"))
        .expect("failed to create scratch dir");
    let out = dir.join(label);
    let mut build_command = Command::new(pycc_bin());
    build_command.args([
        "build",
        py_path.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    if release {
        build_command.arg("--release");
    }
    let status = build_command.status().unwrap();
    assert!(
        status.success(),
        "`pycc build` ({profile}) failed for {label}"
    );
    let pycc_output = Command::new(&out).output().unwrap();
    assert!(
        pycc_output.status.success(),
        "compiled {label} binary ({profile}) exited non-zero"
    );

    let cpython_output = Command::new(oracle_python_bin())
        .arg(py_path)
        .output()
        .unwrap();
    assert!(
        cpython_output.status.success(),
        "CPython oracle exited non-zero for {label}"
    );

    (
        pycc_output.stdout,
        strip_windows_newline_translation(cpython_output.stdout),
    )
}

#[test]
fn oracle_binary_name_appends_the_exe_extension_only_for_windows() {
    assert_eq!(oracle_binary_name(true), "python3.14.exe");
    assert_eq!(oracle_binary_name(false), "python3.14");
}

#[test]
fn strip_windows_newline_translation_removes_cr_before_lf_only() {
    let input = b"line one\r\nline two\nline three\r\n".to_vec();
    let expected = b"line one\nline two\nline three\n".to_vec();
    assert_eq!(strip_windows_newline_translation(input), expected);

    // A lone `\r` not immediately followed by `\n` is not CPython's Windows
    // newline translation and must be left untouched.
    let lone_cr = b"a\rb\n".to_vec();
    assert_eq!(strip_windows_newline_translation(lone_cr.clone()), lone_cr);
}

// Ignored by default: this test shells out to a real "python3.14" oracle,
// which the D-014 coverage gate's isolated `nobody` sandbox deliberately does
// not provision (installing a network-fetched interpreter inside that
// trust boundary would expand it far beyond what the gate is reviewed for).
// CI runs these explicitly with `-- --include-ignored` in a step placed
// after that sandboxed gate, on every Tier-1 target (D-085/D-080).
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn fib_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/conformance_fib.py");
    let (pycc_stdout, cpython_stdout) = run_conformance_fixture("conformance_fib", &fixture);
    assert_eq!(
        pycc_stdout, cpython_stdout,
        "pycc and CPython 3.14.7 disagree on tests/fixtures/conformance_fib.py"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn mandelbrot_ascii_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/conformance_mandelbrot.py");
    let (pycc_stdout, cpython_stdout) = run_conformance_fixture("conformance_mandelbrot", &fixture);
    assert_eq!(
        pycc_stdout, cpython_stdout,
        "pycc and CPython 3.14.7 disagree on tests/fixtures/conformance_mandelbrot.py"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn run_conformance_fixture_with_profile_builds_both_debug_and_release() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/conformance_fib.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("profile_check_debug", &fixture, false);
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("profile_check_release", &fixture, true);
    assert_eq!(
        debug_pycc, debug_cpython,
        "debug profile must match CPython"
    );
    assert_eq!(
        release_pycc, release_cpython,
        "release profile must match CPython"
    );
    assert_eq!(
        debug_pycc, release_pycc,
        "debug and release builds of the same fixture must produce identical stdout"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0526_var_annotations_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0526_var_annotations.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0526_var_annotations_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0526_var_annotations.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0526_var_annotations_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0526_var_annotations.py"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_3105_print_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_3105_print.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_3105_print_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_3105_print.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_3105_print_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_3105_print.py"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_3107_annotations_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_3107_annotations.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_3107_annotations_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_3107_annotations.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_3107_annotations_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_3107_annotations.py"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_3131_unicode_ids_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_3131_unicode_ids.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_3131_unicode_ids_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_3131_unicode_ids.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_3131_unicode_ids_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_3131_unicode_ids.py"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0414_u_literal_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0414_u_literal.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0414_u_literal_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0414_u_literal.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0414_u_literal_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0414_u_literal.py"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0484_type_hints_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0484_type_hints.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0484_type_hints_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0484_type_hints.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0484_type_hints_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0484_type_hints.py"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0498_fstrings_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0498_fstrings.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0498_fstrings_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0498_fstrings.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0498_fstrings_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0498_fstrings.py"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0585_builtin_generics_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0585_builtin_generics.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0585_builtin_generics_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0585_builtin_generics.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0585_builtin_generics_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0585_builtin_generics.py"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0604_union_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0604_union.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0604_union_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0604_union.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0604_union_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0604_union.py"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0572_walrus_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0572_walrus.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0572_walrus_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0572_walrus.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0572_walrus_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0572_walrus.py"
    );
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn dict_order_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dict_order.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("dict_order_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/dict_order.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("dict_order_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/dict_order.py"
    );
}

// `set[int]`'s own iteration order is this implementation's insertion order,
// which does not match CPython's hash-dependent set iteration order (D-123) --
// so, unlike every other fixture in this file, this one intentionally prints
// only an order-independent value (`len()` of a literal with a duplicate
// element) rather than iterating the set. It exists to give the PEP 585
// conformance-matrix row (`docs/PYTHON_STANDARDS.md`) real CI-verified
// evidence for `set[int]`'s own half of that row, since `dict_order.py`
// above only exercises `dict[str, int]`.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0585_set_int_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0585_set_int.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0585_set_int_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0585_set_int.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0585_set_int_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0585_set_int.py"
    );
}

// `tuple[...]`'s own v0.2 slice (D-115/D-116, PR-11b): heterogeneous literal
// construction, `t[k]` literal-index reads across all 3 accepted element
// types (`int`/`bool`/`float`), both storage routes (a module-global tuple
// and a function-local tuple reached through its own alloca slot), and an
// inline (unnamed) tuple subscript. This deliberately does NOT exercise a
// function returning or accepting a tuple *value* itself (as opposed to a
// tuple living entirely inside one function's locals): `pycc_types`'
// private-helper signature-inference solver is scalar-only by construction
// (`collect_expr_constraints` returns no unification term for any container
// literal), so an entirely unannotated helper cannot have its parameter or
// return type inferred as `Ty::Tuple` from real source today -- confirmed
// empirically, and true for `list`/`dict`/`set` the same way, not a
// tuple-specific gap. See the D-116 point 4 correction note in
// `docs/decisions/D-116-tuple-v0-2-scope-int-bool-float-elements-only.md`
// and `docs/ROADMAP.md`'s matching follow-up. Unlike `pep_0585_set_int.py`
// above, this fixture's output is fully order-independent already -- tuples
// have no iteration order question the way sets do -- so it asserts
// byte-for-byte agreement like every other fixture in this file, not just
// `len()`.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn tuple_heterogeneous_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tuple_heterogeneous.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("tuple_heterogeneous_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/tuple_heterogeneous.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("tuple_heterogeneous_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/tuple_heterogeneous.py"
    );
}

// PR-12 Task 13 (D-117/D-120): the PEP-709 conformance fixture. pycc has no
// bytecode/frame model to "inline" the way CPython's own PEP 709 change
// does, so this asserts the one CPython-observable, statically-testable
// guarantee PEP 709 depends on instead -- a comprehension's own loop
// variable does not leak into or clobber an enclosing same-named binding,
// now genuinely exercised for the first time by D-117's synthesized-name
// mechanism. See the D-120 entry,
// `docs/decisions/D-120-the-pep-709-fixture-demonstrates-loop-variable.md`,
// for the full account of why this fixture's exact shape was chosen and
// what CPython actually prints for it.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0709_comp_inline_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0709_comp_inline.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0709_comp_inline_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0709_comp_inline.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0709_comp_inline_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0709_comp_inline.py"
    );
}

// PR-13 Task 5 (D-135): PEP 695 generic-function fixture. One type
// parameter, called at 3 sites across two scalar types (`int` and `str`),
// each result printed -- exercises call-site monomorphization end to end.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0695_generics_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0695_generics.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0695_generics_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0695_generics.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0695_generics_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0695_generics.py"
    );
}

// PR-13 Task 5 (D-135): legacy `TypeAlias` fixture (PEP 613). Deliberately
// omits `from typing import TypeAlias`, unlike the brief's literal text --
// see the fixture file's own comment and this task's report for why: pycc
// has no `Stmt::Import`/`Stmt::ImportFrom` support at all (any `import`
// statement is unconditionally rejected with `C0001`, confirmed against
// this exact fixture), and CPython 3.14 defers annotation evaluation by
// default (PEP 649/749), so the bare `TypeAlias` name in `IntAlias:
// TypeAlias = int` is never evaluated by the pinned 3.14.7 oracle either --
// both sides produce identical output with or without the import.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0613_typealias_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0613_typealias.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0613_typealias_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0613_typealias.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0613_typealias_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0613_typealias.py"
    );
}

// PR-12 Task 13 (D-117/D-118/D-119): a single breadth fixture covering
// `list[int]` slicing, `.pop()`, `dict.get()`, `set.add()`, and both
// comprehension source-container combinations (a `range()`-sourced dict
// comprehension, a `range()`-sourced set comprehension with an `if` filter)
// not already covered by `pep_0709_comp_inline.py` above. Every value is
// printed element-wise or via `len()`, never a container value directly
// (`print(xs)` panics today, D-107/D-124/D-116).
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn container_methods_slicing_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/container_methods_slicing.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("container_methods_slicing_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/container_methods_slicing.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("container_methods_slicing_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/container_methods_slicing.py"
    );
}

// D-139: the hand-authored container/generics differential corpus -- one
// fixture combining a `list[int]`/`dict[str, int]`/`set[int]`/
// `tuple[int, int]` literal (D-116 scopes tuple element compilation to
// int/bool/float, so this uses `(7, 8)`, not D-139's own illustrative
// `tuple[int, str]` text -- a plan-deviation narrowing recorded here and
// on D-139 itself, not silently), a list comprehension and a dict
// comprehension (PEP 709 scoping), `list[int]` slicing, one use each of
// `.pop()`/`.get()`/`.add()`, and a one-type-parameter generic function
// (D-133/D-134) applied to both an `int` and a `str` value -- proving
// these already-shipped v0.2 features compose in a single program, not
// re-testing any single feature in isolation. Every value is printed
// element-wise/via indexing/`len()`, never a container value directly
// (matching `container_methods_slicing_matches_cpython_3_14_7_byte_for_byte`'s
// own established precedent immediately above: `print(xs)` panics today,
// D-107/D-124/D-116).
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn v0_2_container_generics_corpus_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/v0_2_container_generics_corpus.py");
    let (debug_pycc, debug_cpython) = run_conformance_fixture_with_profile(
        "v0_2_container_generics_corpus_debug",
        &fixture,
        false,
    );
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/v0_2_container_generics_corpus.py"
    );
    let (release_pycc, release_cpython) = run_conformance_fixture_with_profile(
        "v0_2_container_generics_corpus_release",
        &fixture,
        true,
    );
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/v0_2_container_generics_corpus.py"
    );
}

// D-138: the PEP-594 (dead-battery stdlib removals) conformance fixture
// pair. `pep_0594_dead_battery.py` proves import resolution genuinely
// works now (a passing `import math` fixture using both registered
// symbols), oracle-diffed dual-profile like every other
// `*_matches_cpython_3_14_7_byte_for_byte` test above.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0594_dead_battery_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0594_dead_battery.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0594_dead_battery_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0594_dead_battery.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0594_dead_battery_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0594_dead_battery.py"
    );
}

// D-138: the second half of the PEP-594 pair -- `cgi` (a real,
// removed-in-3.13 stdlib module, absent from pycc_std's registry) must be
// cleanly rejected with C0001, not silently accepted or a panic.
//
// The fixture first spelled it `import cgi as c` rather than D-138's original
// plain `import cgi`. Part 1 of #1026 gave the plain, unaliased,
// undotted form a meaning: it binds an opaque CPython module object
// (`Ty::Object`) for `pycc build --ext`, so it no longer reaches
// `C0001` at all -- what it reaches instead is `ModuleNotFoundError` in
// the host at module-exec time, which is exactly what CPython 3.14 does
// with the same line, and which
// `tests/issue_1080_foreign_object.rs` pins directly. The aliased form
// `import cgi as c` then carried the fixture until #1291 admitted an
// undotted alias on the same terms as the plain form, and
// `from cgi import escape` carried it until #1278 admitted an unaliased
// from-import of an undotted foreign module.
//
// The fixture is now `from cgi import escape as esc`. Stated honestly, it
// no longer pins a dead-battery property: every non-`pycc_std` root
// reaches the host's `ModuleNotFoundError` when it is missing, as in
// CPython. What it pins is narrower -- an aliased `from` import of a
// module outside `pycc_std` is refused with `C0001`, so `cgi` is never
// silently treated as a *pycc_std* module. See the dated amendments on
// D-137 and D-138.
//
// No
// CPython oracle involved (per D-138's own Context: CPython 3.14 raises
// `ModuleNotFoundError` for this exact source, a fundamentally different
// failure shape than pycc's static C0001 -- the two compilers are
// asserting different things about the same removed module, so
// byte-for-byte comparison would be misleading here, unlike every other
// test in this file). Does not need the `python3.14` oracle on PATH, so
// unlike every other fixture test in this file it runs by default (no
// `#[ignore]`).
#[test]
fn pep_0594_dead_battery_rejected_produces_c0001() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pep_0594_dead_battery_rejected.py");
    let output = Command::new(pycc_bin())
        .args(["check", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "`pycc check` should reject tests/fixtures/pep_0594_dead_battery_rejected.py"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("C0001"),
        "expected a C0001 diagnostic, got: {stdout}"
    );
    assert!(
        stdout.contains("cgi"),
        "expected the diagnostic to mention `cgi`, got: {stdout}"
    );
}

// PEP 698 (#579, Part 3 of #572): `@override` -- its fixture test
// (`pep_0698_override_matches_cpython_3_14_7_byte_for_byte`) lives in
// tests/conformance/classes.rs.

// PEP 593 (#383): `Annotated[X, ...]` — unwraps to `X`, metadata discarded.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0593_annotated_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0593_annotated.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0593_annotated_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0593_annotated.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0593_annotated_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0593_annotated.py"
    );
}

// PEP 570 (#383): positional-only parameters (`/` marker).
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0570_pos_only_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0570_pos_only.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0570_pos_only_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0570_pos_only.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0570_pos_only_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0570_pos_only.py"
    );
}

// PEP 570, enforcement half (Part 1 of #884, #1125): naming a
// positional-only parameter in a keyword argument is a `T0021` static
// rejection. No CPython oracle is involved, for the same reason as
// `pep_0594_dead_battery_rejected_produces_c0001` above: CPython raises
// `TypeError` at call time while pycc rejects the program before it runs,
// so a byte-for-byte comparison would compare two different failure
// shapes. Needs no `python3.14` oracle on PATH, so it runs by default
// (no `#[ignore]`).
#[test]
fn pep_0570_pos_only_keyword_rejected_produces_t0021() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pep_0570_pos_only_keyword_rejected.py");
    let output = Command::new(pycc_bin())
        .args(["check", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "`pycc check` should reject tests/fixtures/pep_0570_pos_only_keyword_rejected.py"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("T0021"),
        "expected a T0021 diagnostic, got: {stdout}"
    );
    assert!(
        stdout.contains("positional-only parameter `a`"),
        "expected the diagnostic to name the positional-only parameter, got: {stdout}"
    );
}

// PEP 591 (#383): `Final[X]` — unwraps to `X`, non-reassignable.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0591_final_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0591_final.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0591_final_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0591_final.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0591_final_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0591_final.py"
    );
}

// PEP 634-636 (#381, PR-21): structural pattern matching (`match`/`case`).
// Literal, singleton, capture, wildcard, guard, and or-pattern forms.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0634_match_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0634_match.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0634_match_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0634_match.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0634_match_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0634_match.py"
    );
}

// #575 (Part 2 of #123): `str * int` / `int * str` repetition. Covers both
// operand orders for literal, variable, boolean, zero, and negative counts,
// a repetition result that crosses D-059's 22-byte inline payload threshold,
// and repetition composed with concatenation, an f-string, and a function
// return. Negative counts are spelled `0 - 2` rather than `-2`: the fixture
// predates #602's literal-sign fold, and the `BinOp::Sub` form remains a
// valid, equally reachable way to produce a negative `int`.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn str_repetition_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/str_repetition.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("str_repetition_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/str_repetition.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("str_repetition_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/str_repetition.py"
    );
}

// #719 (PEP 701): the formalized f-string grammar. Covers nested same-quote
// f-strings, nesting to depth 5, single quotes inside a double-quoted
// f-string and the all-single-quote form, multi-line interpolation
// expressions, a `#` comment inside a multi-line interpolation, `\N{...}`
// named escapes both inside an interpolation and in a literal part, adjacent
// interpolations, escaped braces, raw f-strings, and implicit adjacent
// f-string concatenation. Conversion flags, format specs and the `=` debug
// specifier (#720) are deliberately absent -- they are recorded core gaps in
// pycc's f-string lowering rather than PEP 701 grammar points.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0701_fstring_grammar_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0701_fstring_grammar.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0701_fstring_grammar_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0701_fstring_grammar.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0701_fstring_grammar_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0701_fstring_grammar.py"
    );
}

// Part 1 of #883 (#962): `import math as m` -- the aliased receiver reaches
// the same libm calls and constant as the canonical spelling, at module
// level and inside a function body, oracle-diffed dual-profile like the
// `pep_0594_dead_battery` positive fixture.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn import_alias_math_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/conformance_import_alias_math.py");
    let (debug_pycc, debug_cpython) = run_conformance_fixture_with_profile(
        "conformance_import_alias_math_debug",
        &fixture,
        false,
    );
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/conformance_import_alias_math.py"
    );
    let (release_pycc, release_cpython) = run_conformance_fixture_with_profile(
        "conformance_import_alias_math_release",
        &fixture,
        true,
    );
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/conformance_import_alias_math.py"
    );
}
