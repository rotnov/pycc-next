//! The release archive works on its own (#5): `scripts/package.sh` lays out
//! `bin/pycc` + `lib/pycc/libpycc_rt.a` in a temp directory outside the
//! repository, the directory is then moved, and that copy -- run with a
//! cleared environment (no `CARGO_*` variables, an empty `HOME`) -- builds
//! programs that match CPython. `v0.1.0-rc.1` shipped only the binary, which
//! looked for its runtime under the CI runner's checkout path.
//!
//! The release workflow runs the same layout through `scripts/smoke.sh`
//! before publishing.

#![cfg(unix)]

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Packages the binary under test into `<scratch>/staging`, then moves it to
/// `<scratch>/moved` so nothing can depend on where it was first written.
fn packaged(scratch: &Path) -> PathBuf {
    let profile = Path::new(env!("CARGO_BIN_EXE_pycc")).parent().unwrap();
    let staging = scratch.join("staging");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/package.sh");
    let output = Command::new("sh")
        .arg(&script)
        .arg(&staging)
        .arg(profile)
        .output()
        .expect("run package.sh");
    assert!(output.status.success(), "{}", stderr(&output));
    let moved = scratch.join("moved");
    std::fs::rename(&staging, &moved).unwrap();
    for file in [
        "bin/pycc",
        "lib/pycc/libpycc_rt.a",
        "README.md",
        "LICENSE",
        "CHANGELOG.md",
    ] {
        assert!(moved.join(file).is_file(), "the archive lacks {file}");
    }
    let backend = format!("pycc_build-{}-py3-none-any.whl", env!("CARGO_PKG_VERSION"));
    assert!(
        moved.join("python").join(&backend).is_file(),
        "the archive lacks python/{backend}"
    );
    moved
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn python() -> String {
    std::env::var("PYCC_PYTHON").unwrap_or_else(|_| "python3".to_string())
}

/// A command with only what a user's shell would have: `PATH` and an empty
/// `HOME` (plus `PYCC_PYTHON` when the test run names an interpreter).
fn isolated(program: impl AsRef<std::ffi::OsStr>, scratch: &Path, cwd: &Path) -> Command {
    let home = scratch.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut cmd = Command::new(program);
    cmd.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .current_dir(cwd);
    if let Some(python) = std::env::var_os("PYCC_PYTHON") {
        cmd.env("PYCC_PYTHON", python);
    }
    cmd
}

fn run_ok(cmd: &mut Command) -> Vec<u8> {
    let output = cmd.output().expect("spawn");
    assert!(
        output.status.success(),
        "{:?} failed: {}",
        cmd,
        stderr(&output)
    );
    output.stdout
}

fn work_dir(scratch: &Path, name: &str, file: &str, source: &str) -> PathBuf {
    let dir = scratch.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(file), source).unwrap();
    dir
}

const FIB: &str = "\
def fib(n: int) -> int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


print(fib(27))
";

#[test]
fn the_packaged_compiler_builds_fib_natively_from_anywhere() {
    let scratch = ScratchDir::new("relocatable_native").expect("scratch");
    let pkg = packaged(&scratch);
    let pycc = pkg.join("bin/pycc");
    let dir = work_dir(&scratch, "native", "fib.py", FIB);
    let oracle = run_ok(Command::new(python()).arg(dir.join("fib.py")));

    for (out, extra) in [("fib_release", &["--release"][..]), ("fib_debug", &[][..])] {
        run_ok(
            isolated(&pycc, &scratch, &dir)
                .args(["build", "fib.py", "-o", out])
                .args(extra),
        );
        assert_eq!(run_ok(&mut Command::new(dir.join(out))), oracle, "{out}");
    }
}

/// Without its runtime archive the installed compiler fails cleanly and
/// names where the archive belongs -- it never falls back to the checkout
/// it was built from, which is what made `v0.1.0-rc.1` look fine on the
/// runner that built it.
#[test]
fn the_packaged_compiler_without_its_runtime_names_the_missing_archive() {
    let scratch = ScratchDir::new("relocatable_no_rt").expect("scratch");
    let pkg = packaged(&scratch);
    std::fs::remove_file(pkg.join("lib/pycc/libpycc_rt.a")).unwrap();
    let dir = work_dir(&scratch, "native", "fib.py", FIB);
    let output = isolated(pkg.join("bin/pycc"), &scratch, &dir)
        .args(["build", "fib.py", "-o", "fib"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    let expected = pkg.join("lib/pycc/libpycc_rt.a");
    assert!(
        stderr(&output).contains(&format!("expected {} beside this pycc", expected.display())),
        "{}",
        stderr(&output)
    );
    assert!(!dir.join("fib").exists());
}

const EXT: &str = "\
def add(a: int, b: int) -> int:
    return a + b


def greet(name: str) -> str:
    return 'hello ' + name


def mean(xs: tuple[float, float, float]) -> float:
    return (xs[0] + xs[1] + xs[2]) / 3
";

const EXT_DRIVER: &str = "import m; print(m.add(2, 40), m.greet('pycc'), m.mean((1.0, 2.0, 4.5)))";

const NON_STRICT: &str = "\
def squares(n):
    for i in range(n):
        yield i * i


inc = lambda x: x + 1
total = sum(squares(10))
print(total, inc(41))
try:
    print(int('nope'))
except ValueError as e:
    print('ValueError:', e)
";

/// `--ext` and `--non-strict` (an embedded CPython) from the packaged
/// compiler, compared with CPython. Ignored by default like the repository's
/// other embedded-interpreter tests: it needs CPython 3.13+ with its headers
/// and a shared libpython (`--include-ignored`, as CI runs).
#[test]
#[ignore = "needs CPython 3.13+ with headers and a shared libpython"]
fn the_packaged_compiler_builds_ext_and_non_strict_programs() {
    let scratch = ScratchDir::new("relocatable_python").expect("scratch");
    let pkg = packaged(&scratch);
    let pycc = pkg.join("bin/pycc");

    let source = work_dir(&scratch, "ext_src", "m.py", EXT);
    let built = work_dir(&scratch, "ext_built", "m.py", EXT);
    run_ok(isolated(&pycc, &scratch, &built).args([
        "build",
        "m.py",
        "-o",
        "m",
        "--ext",
        "--release",
    ]));
    std::fs::remove_file(built.join("m.py")).unwrap();
    let oracle = run_ok(isolated(python(), &scratch, &source).args(["-c", EXT_DRIVER]));
    let compiled = run_ok(isolated(python(), &scratch, &built).args(["-c", EXT_DRIVER]));
    assert_eq!(compiled, oracle);

    let dir = work_dir(&scratch, "non_strict", "prog.py", NON_STRICT);
    let oracle = run_ok(Command::new(python()).arg(dir.join("prog.py")));
    run_ok(isolated(&pycc, &scratch, &dir).args([
        "build",
        "prog.py",
        "-o",
        "prog",
        "--non-strict",
    ]));
    assert_eq!(
        run_ok(&mut isolated(dir.join("prog"), &scratch, &dir)),
        oracle
    );
}
