//! Non-strict mode (#3): untyped and partially typed Python compiles, with
//! what cannot be proven typed held as a CPython object and run through the
//! C API, so it matches CPython exactly. Strict stays the default.
//!
//! The native arm builds an embedded executable (an object needs an
//! interpreter), which bundles the CPython `PYCC_PYTHON` names (default
//! `python3`); it is skipped with a note when there is none. The
//! `--ext` arm imports the module into `python3`, the oracle.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::Command;

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

/// Whether an embedded build can find its CPython.
fn embed_interpreter_available() -> bool {
    let interpreter = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    Command::new(interpreter)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn copy_into(dir: &Path, fixture_name: &str, as_name: &str) -> PathBuf {
    let path = dir.join(as_name);
    std::fs::copy(fixture(fixture_name), &path).expect("copy the fixture");
    path
}

#[test]
fn untyped_program_matches_cpython_in_a_native_build() {
    if !embed_interpreter_available() {
        eprintln!("skipped: no CPython to embed (set PYCC_PYTHON)");
        return;
    }
    let dir = ScratchDir::new("non_strict_native").expect("scratch");
    let source = copy_into(&dir, "non_strict_untyped.py", "m.py");
    let build = pycc()
        .args(["build", "--non-strict"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let ours = Command::new(dir.join("app"))
        .output()
        .expect("run the build");
    let oracle = Command::new("python3")
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert_eq!(text(&ours.stdout), text(&oracle.stdout));
    assert_eq!(ours.status.code(), Some(1), "{}", text(&ours.stderr));
    assert_eq!(oracle.status.code(), Some(1));
    assert_eq!(last_line(&ours.stderr), last_line(&oracle.stderr));
}

#[test]
fn the_same_program_is_rejected_in_strict_mode() {
    let output = pycc()
        .arg("check")
        .arg(fixture("non_strict_untyped.py"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1));
    let rendered = text(&output.stdout);
    assert!(
        rendered.contains(
            "error[T0001]: parameter `a` of public function `add` needs a type annotation"
        ),
        "{rendered}"
    );
}

#[test]
fn non_strict_check_accepts_the_program() {
    let output = pycc()
        .args(["check", "--non-strict"])
        .arg(fixture("non_strict_untyped.py"))
        .output()
        .expect("pycc should spawn");
    assert!(output.status.success(), "{}", text(&output.stdout));
}

/// `[tool.pycc] strict = false` in a neighbouring `pyproject.toml` selects
/// the mode without the flag; `[build] strict = false` in a `pycc.toml` too.
#[test]
fn a_project_file_next_to_the_entry_selects_non_strict() {
    for (file, contents) in [
        ("pyproject.toml", "[tool.pycc]\nstrict = false\n"),
        ("pycc.toml", "[build]\nstrict = false\n"),
    ] {
        let dir = ScratchDir::new("non_strict_config").expect("scratch");
        let source = copy_into(&dir, "non_strict_ext.py", "m.py");
        let strict = pycc().arg("check").arg(&source).output().expect("spawn");
        assert_eq!(strict.status.code(), Some(1), "{file}");
        std::fs::write(dir.join(file), contents).expect("write the project file");
        let output = pycc().arg("check").arg(&source).output().expect("spawn");
        assert!(output.status.success(), "{file}: {}", text(&output.stdout));
    }
}

const EXT_DRIVER: &str = r#"
import m
print(m.add(1, 2), m.add("a", "b"), m.add(1.5, 2), m.add([1], [2]))
print(m.calls_typed(3))
print(m.total([1, 2, 3]), m.total([0.5, 0.25]), m.total(range(4)))
print(m.record({}, "k", None), m.record({1: 2}, (1, 2), [3]))
print(m.forget({"a": 1, "b": 2}, "a"))
print(m.pair(1, 2), m.pair("x", "y"))
xs = [1]
print(m.extend(xs, [2]), xs)
for args in [(1, "a"), ("a", 1), (None, 1)]:
    try:
        m.add(*args)
    except TypeError as e:
        print("TypeError:", e)
try:
    m.ratio(1, 0)
except ZeroDivisionError as e:
    print("ZeroDivisionError:", e)
try:
    m.forget({}, "x")
except KeyError as e:
    print("KeyError:", e)
try:
    m.calls_typed("s")
except TypeError:
    print("TypeError")
"#;

#[test]
fn untyped_ext_module_matches_its_source() {
    let compiled = ScratchDir::new("non_strict_ext").expect("scratch");
    let source = copy_into(&compiled, "non_strict_ext.py", "m.py");
    let build = pycc()
        .args(["build", "--non-strict", "--ext"])
        .arg(&source)
        .arg("-o")
        .arg(compiled.join("m"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new("non_strict_ext_src").expect("scratch");
    copy_into(&plain, "non_strict_ext.py", "m.py");
    let run = |dir: &Path| {
        Command::new("python3")
            .args(["-c", EXT_DRIVER])
            .current_dir(dir)
            .output()
            .expect("python3 should spawn")
    };
    let ours = run(&compiled);
    let oracle = run(&plain);
    assert!(ours.status.success(), "{}", text(&ours.stderr));
    assert_eq!(text(&ours.stdout), text(&oracle.stdout));
}

/// What non-strict mode cannot compile yet falls back to CPython (#12,
/// `tests/non_strict_fallback.rs`) with `PYCC_NO_FALLBACK` unset; with it
/// set, it stays a located refusal, never a miscompilation.
#[test]
fn an_unsupported_construct_is_a_c0001_without_the_fallback() {
    let dir = ScratchDir::new("non_strict_c0001").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(&source, "def f(x):\n    return lambda: x\n").expect("write");
    let output = pycc()
        .args(["check", "--non-strict"])
        .arg(&source)
        .env("PYCC_NO_FALLBACK", "1")
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stdout).contains("error[C0001]"),
        "{}",
        text(&output.stdout)
    );
    let fallback = pycc()
        .args(["check", "--non-strict"])
        .arg(&source)
        .output()
        .expect("pycc should spawn");
    assert!(fallback.status.success(), "{}", text(&fallback.stdout));
}
