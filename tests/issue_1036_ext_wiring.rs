//! `pycc build --ext` end to end, through the real CLI (D-244, #1036,
//! Part 1 of #1025).
//!
//! The non-`#[ignore]`d tests here need no CPython installation: they drive
//! the flag through `main()`'s own `Command::Build` arm and assert the
//! diagnostics and exit codes `docs/CLI_SPEC.md` promises. That matters for
//! more than tidiness -- `.github/workflows/ci.yml`'s coverage job runs
//! `llvm-cov` without `--include-ignored`, so an ignored test contributes
//! exactly zero coverage while its lines stay in
//! `scripts/check_diff_coverage.py`'s denominator.
//!
//! The `#[ignore]`d test at the end is the real thing: it builds an artifact
//! against an installed CPython and imports it. It is opt-in because it
//! needs a `python3` with development headers, which is a property of the
//! machine, not of the change under test. CI installs CPython 3.13 -- D-244's
//! stable-ABI floor -- on every Tier-1 leg of `native-build-test` and runs it
//! there through that job's existing `cargo test --workspace --
//! --include-ignored`.
//!
//! `PYCC_PYTHON`/`PYCC_PYTHON_INCLUDE` are set on the *child* `Command`
//! here, never on this test process: `std::env::set_var` is process-global
//! and would race every other test in this binary.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn write(dir: &Path, body: &str) -> std::path::PathBuf {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    src
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// An include directory that exists but holds no `Python.h`, so the build
/// fails deterministically on any host -- in the toolchain probe, which
/// rejects a header directory without the header rather than letting the C
/// compiler report a broken CPython installation as a compile error.
fn header_less_build(dir: &Path, src: &Path, out: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(src)
        .arg("-o")
        .arg(out)
        .arg("--ext")
        .env("PYCC_PYTHON_INCLUDE", dir)
        .output()
        .expect("pycc should spawn")
}

#[test]
fn ext_rejects_a_public_function_the_boundary_cannot_carry_with_c0003() {
    let dir = ScratchDir::new("ext_cli_gap").expect("scratch");
    // `list[int]` is still outside the boundary after #1048 widened it to
    // the scalars, #1049 added `str` and #1050 added a scalar-element
    // `tuple`. It is not merely the next part's work: a `tuple` crosses by
    // value into one slot per element, and a `list` has no such crossing.
    let src = write(
        &dir,
        "def greet(who: list[int]) -> list[int]:\n    return who\n",
    );
    let output = header_less_build(&dir, &src, &dir.join("m"));
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert!(stderr.contains("error[C0003]"), "{stderr}");
    assert!(stderr.contains("`greet`"), "{stderr}");
    // The advice is actionable and names both ways out.
    assert!(stderr.contains("`_greet`"), "{stderr}");
    assert!(stderr.contains("without --ext"), "{stderr}");
}

#[test]
fn ext_reports_every_capability_gap_in_one_build() {
    let dir = ScratchDir::new("ext_cli_gaps").expect("scratch");
    let src = write(
        &dir,
        "def a(x: list[int]) -> int:\n    return 1\n\n\
         def b(y: int) -> list[int]:\n    return [1]\n",
    );
    let output = header_less_build(&dir, &src, &dir.join("m"));
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert_eq!(stderr.matches("error[C0003]").count(), 2, "{stderr}");
}

#[test]
fn a_private_function_is_not_in_the_export_set_and_raises_no_gap() {
    let dir = ScratchDir::new("ext_cli_private").expect("scratch");
    // D-038: the leading underscore is the whole opt-out mechanism.
    let src = write(
        &dir,
        // The fixture must name a type the boundary still cannot carry:
        // with a carriable one the assertion below holds whether or not
        // D-038's opt-out is honoured, and the test proves nothing. `str`
        // stopped serving that role at #1049, so this is a container.
        "def _greet(who: list[int]) -> list[int]:\n    return who\n\n\
         def twice(x: int) -> int:\n    return x * 2\n",
    );
    let output = header_less_build(&dir, &src, &dir.join("m"));
    let stderr = stderr_of(&output);
    assert!(!stderr.contains("C0003"), "{stderr}");
    // It got past the export scan and died in the toolchain probe instead,
    // which is the expected end of the road without CPython headers -- and an
    // environment failure, not a compile error.
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("Python.h"), "{stderr}");
}

#[test]
fn an_ext_output_path_that_names_no_module_is_an_invocation_failure() {
    let dir = ScratchDir::new("ext_cli_out").expect("scratch");
    let src = write(&dir, "def f() -> int:\n    return 1\n");
    let output = header_less_build(&dir, &src, Path::new("/"));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr_of(&output).contains("--ext output path"),
        "{output:?}"
    );
}

#[test]
fn an_ext_output_path_with_an_interpreter_specific_suffix_is_rejected() {
    let dir = ScratchDir::new("ext_cli_tagged").expect("scratch");
    let src = write(&dir, "def f() -> int:\n    return 1\n");
    // "Interpreter-specific" is a property of the *host's own* suffix family:
    // the rejection fires when the name already ends in that family's tagged
    // tail. A Linux-tagged `.so` name is not tagged at all on Windows, where
    // the family's tail is `.pyd` -- there it would simply gain a `.pyd` and
    // build. Pick the name the running host actually recognizes.
    let tagged = if cfg!(windows) {
        "m.cp313-win_amd64.pyd"
    } else {
        "m.cpython-314-x86_64-linux-gnu.so"
    };
    let out = dir.join(tagged);
    let output = header_less_build(&dir, &src, &out);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr_of(&output).contains("interpreter-specific"),
        "{output:?}"
    );
}

#[test]
fn an_unrunnable_interpreter_is_an_environment_failure_naming_the_override() {
    let dir = ScratchDir::new("ext_cli_interp").expect("scratch");
    let src = write(&dir, "def f() -> int:\n    return 1\n");
    let output = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("m"))
        .arg("--ext")
        .env("PYCC_PYTHON", "pycc-no-such-interpreter-1036")
        .env_remove("PYCC_PYTHON_INCLUDE")
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(2));
    let stderr = stderr_of(&output);
    assert!(stderr.contains("pycc-no-such-interpreter-1036"), "{stderr}");
    assert!(stderr.contains("PYCC_PYTHON_INCLUDE"), "{stderr}");
}

#[test]
fn a_native_build_of_the_same_source_is_unaffected_by_the_new_flag() {
    // The regression guard for the whole change: `--ext` must not have
    // altered the native path it was threaded through.
    let dir = ScratchDir::new("ext_cli_native").expect("scratch");
    let src = write(
        &dir,
        "def twice(x: int) -> int:\n    return x * 2\n\nprint(twice(21))\n",
    );
    let out = dir.join("native");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let run = Command::new(&out).output().expect("the binary should run");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "42\n");
}

/// The three review findings whose symptoms only exist in a *loaded*
/// artifact, in one build so they cost one compile between them.
///
/// `#[ignore]`d for the same reason as the oracle below -- it needs a
/// CPython with development headers -- and, like it, run on every Tier-1
/// `native-build-test` leg through that job's `--include-ignored`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_built_ext_module_keeps_its_globals_collapses_a_rebind_and_preserves_oserror() {
    let dir = ScratchDir::new("ext_oracle_review").expect("scratch");
    let src = write(
        &dir,
        // A module-level `str` read back through an export. Native mode
        // decrefs these globals when `main` returns; `Py_mod_exec` is not a
        // process exit, so doing it here freed storage the wrappers still
        // read -- a segfault under a checking allocator and a wrong answer
        // under the ordinary one, which is what the second call catches.
        "greeting = \"hello world\"\n\n\
         def size() -> int:\n    t = greeting\n    if t == \"hello world\":\n        return 1\n    return 0\n\n\
         def twice(x: int) -> int:\n    return x\n\n\
         def twice(x: int) -> int:\n    return x * 2\n\n\
         def boom(n: int) -> int:\n    if n == 0:\n        raise FileNotFoundError(\"no such probe\")\n    return n\n",
    );
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_oracle_review"))
        .arg("--ext")
        // The rebind above used to reach the C compiler as a duplicate
        // `pycc_ext_wrap_twice`, so this build failed outright.
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));

    let script = "import pycc_oracle_review as m\n\
                  assert m.size() == 1, m.size()\n\
                  assert m.size() == 1, 'the module global did not survive the first call'\n\
                  assert m.twice(21) == 42, m.twice(21)\n\
                  try:\n    m.boom(0)\n\
                  except FileNotFoundError as e:\n    assert str(e) == 'no such probe', str(e)\n\
                  else:\n    raise AssertionError('the OSError-family class did not cross')\n\
                  print('ok')\n";
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        stderr_of(&run)
    );
}

/// The real artifact, against a real interpreter: builds an extension
/// module and asks CPython to import it and call it.
///
/// `#[ignore]`d because it needs a CPython with development headers on the
/// machine. Everything it proves that the tests above cannot -- that the
/// generated C compiles against a real `Python.h`, that the multi-phase
/// init runs the module body before any wrapper is reachable, and that the
/// `int` boundary round-trips -- is a property of a built artifact, and
/// there is no way to observe it without building one.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_built_ext_module_imports_and_answers_through_the_int_boundary() {
    let dir = ScratchDir::new("ext_oracle").expect("scratch");
    let src = write(
        &dir,
        "def twice(x: int) -> int:\n    return x * 2\n\n\
         def add(a: int, b: int) -> int:\n    return a + b\n\n\
         def zero() -> int:\n    return 0\n",
    );
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_oracle_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));

    let script = "import sys, pycc_oracle_mod as m\n\
                  assert m.twice(21) == 42, m.twice(21)\n\
                  assert m.add(1, 2) == 3\n\
                  assert m.zero() == 0\n\
                  assert m.twice(True) == 2\n\
                  try:\n    m.twice('x')\nexcept TypeError:\n    pass\n\
                  else:\n    raise AssertionError('str must not be accepted')\n\
                  try:\n    m.twice(1, 2)\nexcept TypeError:\n    pass\n\
                  else:\n    raise AssertionError('arity must be checked')\n\
                  assert m.twice(x=4) == 8\n\
                  try:\n    m.twice(y=1)\nexcept TypeError:\n    pass\n\
                  else:\n    raise AssertionError('an unknown keyword must be refused')\n\
                  print('ok')\n";
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        stderr_of(&run)
    );
}
