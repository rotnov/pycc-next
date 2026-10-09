//! #1467: a module-level `__getattr__` / `__dir__` (PEP 562) in the entry
//! module of an `--ext` build is published as a module attribute, so the
//! host calls it exactly as CPython calls the same source module's hook.
//! Before this change D-038's public-name predicate kept every dunder out
//! of D-244 rule 1's export set, and the host silently ignored the hook:
//! the issue's module raised `AttributeError` where CPython printed `42`.
//!
//! [`DRIVER`] runs against the extension and against the same source
//! imported as plain Python, so CPython is the oracle for every line: a
//! missing attribute, `getattr` with a default, `hasattr` over a compiled
//! `raise AttributeError`, `from m import x`, `dir(m)` through `__dir__`,
//! and a re-import after `del sys.modules[...]`. A helper module's hook is
//! not applied to the entry module, on either side.
//!
//! [`EXT_ONLY_DRIVER`] pins the residual D-244's #1467 amendment records:
//! a name CPython's module dict holds but the extension does not publish
//! (here a private function) reaches the hook.
//!
//! The hosted tests are `#[ignore]`d for the reason every `ext` test is:
//! they ask an installed CPython with development headers to build and
//! import the artifact; the Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The refusals are answered
//! before any interpreter is probed, so those tests need no CPython.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

fn build_ext(source: &Path, out: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(out)
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

fn run(script: &str, path_entry: &Path, cwd: &Path) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .env("PYTHONPATH", path_entry)
        .current_dir(cwd)
        .output()
        .expect("python3 should spawn")
}

/// The module under test: the issue's own hook, widened to a compiled
/// `raise AttributeError`, plus a `__dir__` and an ordinary export.
const MODULE: &str = r#"def __getattr__(name: str) -> int:
    if name == "boom":
        raise AttributeError("no boom")
    if name == "zz":
        return 2
    return 42


def __dir__() -> str:
    return "ba"


def _helper() -> int:
    return 7


def f() -> int:
    return _helper()
"#;

const DRIVER: &str = r#"import sys
import pycc_ga_mod as m

print(m.anything)
print(m.zz, m.f())
print(getattr(m, "q", 7), getattr(m, "boom", "default"), hasattr(m, "boom"))
try:
    m.boom
except AttributeError as e:
    print("AttributeError", e, type(e) is AttributeError)
from pycc_ga_mod import xyz
print(xyz)
print(dir(m))
del sys.modules["pycc_ga_mod"]
import pycc_ga_mod as again
print(again is m, again.zz, again.other)
"#;

const DRIVER_OUT: &str = "42\n\
    2 7\n\
    42 default False\n\
    AttributeError no boom True\n\
    42\n\
    ['a', 'b']\n\
    False 2 42\n";

/// Differs from CPython by design: see the module doc.
const EXT_ONLY_DRIVER: &str = r#"import pycc_ga_mod as m

print(m._helper)
"#;

const EXT_ONLY_OUT: &str = "42\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_entry_modules_hooks_are_called_by_the_host_as_cpython_calls_them() {
    let dir = ScratchDir::new("ext_module_getattr").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    let source = write(&oracle, "pycc_ga_mod.py", MODULE);
    let build = build_ext(&source, &ext.join("pycc_ga_mod"));
    assert!(build.status.success(), "{}", stderr_of(&build));
    // Each run's working directory is the scratch root, so neither side can
    // import the module from where it runs.
    let compiled = run(DRIVER, &ext, &dir);
    assert_ok(&compiled);
    let cpython = run(DRIVER, &oracle, &dir);
    assert_ok(&cpython);
    assert_eq!(stdout_of(&compiled), stdout_of(&cpython));
    assert_eq!(stdout_of(&compiled), DRIVER_OUT);
    let divergent = run(EXT_ONLY_DRIVER, &ext, &dir);
    assert_ok(&divergent);
    assert_eq!(stdout_of(&divergent), EXT_ONLY_OUT);
    let cpython_divergent = run(EXT_ONLY_DRIVER, &oracle, &dir);
    assert_ok(&cpython_divergent);
    assert!(
        stdout_of(&cpython_divergent).starts_with("<function _helper"),
        "{}",
        stdout_of(&cpython_divergent)
    );
}

/// A helper module's `def __getattr__` is linked into the same program
/// (D-222) but belongs to the helper: CPython never applies it to the entry
/// module, and neither does the extension.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_helper_modules_hook_is_not_applied_to_the_entry_module() {
    let dir = ScratchDir::new("ext_module_getattr_helper").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    write(
        &oracle,
        "pycc_ga_helper.py",
        "def __getattr__(name: str) -> int:\n    return 1\n\n\ndef one() -> int:\n    return 1\n",
    );
    let source = write(
        &oracle,
        "pycc_ga_entry.py",
        "from pycc_ga_helper import one\n\n\ndef g() -> int:\n    return one() + 1\n",
    );
    let build = build_ext(&source, &ext.join("pycc_ga_entry"));
    assert!(build.status.success(), "{}", stderr_of(&build));
    let script = "import pycc_ga_entry as m\nprint(getattr(m, \"zz\", \"missing\"), m.g())\n";
    let compiled = run(script, &ext, &dir);
    assert_ok(&compiled);
    let cpython = run(script, &oracle, &dir);
    assert_ok(&cpython);
    assert_eq!(stdout_of(&compiled), stdout_of(&cpython));
    assert_eq!(stdout_of(&compiled), "missing 2\n");
}

/// A hook whose return carries a `tuple` crosses through a codegen thunk
/// rather than the `fnptr_` slot, so the codegen mirror and the driver must
/// agree that `__getattr__` is exported; a disagreement emits the wrong C
/// declaration and faults on the first call instead of failing the build.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_tuple_returning_hook_crosses_through_its_thunk() {
    let dir = ScratchDir::new("ext_module_getattr_tuple").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    let source = write(
        &oracle,
        "pycc_ga_tuple.py",
        "def __getattr__(name: str) -> tuple[int, int]:\n    if name == \"abc\":\n        return (3, 7)\n    return (2, 7)\n",
    );
    let build = build_ext(&source, &ext.join("pycc_ga_tuple"));
    assert!(build.status.success(), "{}", stderr_of(&build));
    let script = "import pycc_ga_tuple as m\nprint(m.abc, getattr(m, \"zz\"))\n";
    let compiled = run(script, &ext, &dir);
    assert_ok(&compiled);
    let cpython = run(script, &oracle, &dir);
    assert_ok(&cpython);
    assert_eq!(stdout_of(&compiled), stdout_of(&cpython));
    assert_eq!(stdout_of(&compiled), "(3, 7) (2, 7)\n");
}

/// An import that binds a hook name in the entry module puts a hook in
/// CPython's module dict that the extension cannot publish, so it is
/// refused at its own line instead of being ignored. No CPython is needed:
/// the refusal precedes the toolchain probe.
#[test]
fn an_import_binding_a_hook_name_is_refused_at_its_line() {
    let dir = ScratchDir::new("ext_module_getattr_import").expect("scratch");
    write(
        &dir,
        "pycc_ga_lib.py",
        "def __getattr__(name: str) -> int:\n    return 1\n",
    );
    let source = write(
        &dir,
        "m.py",
        "from pycc_ga_lib import __getattr__\n\n\ndef f() -> int:\n    return 1\n",
    );
    let output = build_ext(&source, &dir.join("m"));
    let stderr = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert_eq!(stderr.matches("error[").count(), 1, "{stderr}");
    assert!(
        stderr.contains(
            "error[C0001]: --ext cannot publish a module `__getattr__` bound by an import"
        ),
        "{stderr}"
    );
    assert!(stderr.contains("m.py:1:25"), "{stderr}");
    assert!(stderr.contains("(#1467)"), "{stderr}");
    assert!(
        !dir.join("m").exists(),
        "a refused build leaves no artifact"
    );
}

/// A module-level assignment to a hook name compiled, and the host then
/// raised `AttributeError` where CPython raises `TypeError: 'int' object is
/// not callable` from the bound value: refused at its target instead.
#[test]
fn an_assignment_binding_a_hook_name_is_refused_at_its_target() {
    let dir = ScratchDir::new("ext_module_getattr_assign").expect("scratch");
    let source = write(
        &dir,
        "m.py",
        "__getattr__ = 5\n\n\ndef f() -> int:\n    return 1\n",
    );
    let output = build_ext(&source, &dir.join("m"));
    let stderr = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert_eq!(stderr.matches("error[").count(), 1, "{stderr}");
    assert!(
        stderr.contains(
            "error[C0001]: --ext cannot publish a module `__getattr__` bound by an assignment"
        ),
        "{stderr}"
    );
    assert!(stderr.contains("m.py:1:1"), "{stderr}");
    assert!(
        !dir.join("m").exists(),
        "a refused build leaves no artifact"
    );
}

/// A hook `def` inside a module-level block binds the hook in CPython only
/// when its branch runs; the extension must not build without it. Today the
/// frontend refuses any `def` nested in a block before the hook scan runs,
/// and the scan refuses it too, so this pins the end-to-end outcome --
/// a located `C0001` and no artifact -- rather than which layer answers.
#[test]
fn a_hook_def_inside_a_module_level_block_is_refused() {
    let dir = ScratchDir::new("ext_module_getattr_nested").expect("scratch");
    for (file, body, location) in [
        (
            "m_if.py",
            "import sys\nif sys.version_info >= (3, 7):\n    def __getattr__(name: str) -> int:\n        return 1\n",
            "m_if.py:3:5",
        ),
        (
            "m_try.py",
            "try:\n    def __dir__() -> str:\n        return \"a\"\nexcept ImportError:\n    pass\n",
            "m_try.py:2:5",
        ),
    ] {
        let source = write(&dir, file, body);
        let out = dir.join(file.trim_end_matches(".py"));
        let output = build_ext(&source, &out);
        let stderr = stderr_of(&output);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(stderr.contains("error[C0001]"), "{stderr}");
        assert!(stderr.contains(location), "{stderr}");
        assert!(!out.exists(), "a refused build leaves no artifact");
    }
}

/// A hook whose signature the boundary cannot carry is a `C0003` whose
/// remedy is not "rename it private": that would silently stop the host
/// from calling it, the defect #1467 removed.
#[test]
fn an_uncarriable_hook_is_a_capability_gap_with_its_own_remedy() {
    let dir = ScratchDir::new("ext_module_getattr_gap").expect("scratch");
    let source = write(
        &dir,
        "m.py",
        "def __getattr__(name: str) -> list[int]:\n    return [1]\n",
    );
    let output = build_ext(&source, &dir.join("m"));
    let stderr = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert_eq!(stderr.matches("error[").count(), 1, "{stderr}");
    assert!(
        stderr.contains("error[C0003]: --ext cannot publish the module hook `__getattr__`"),
        "{stderr}"
    );
    assert!(stderr.contains("change its signature"), "{stderr}");
    assert!(!stderr.contains("rename"), "{stderr}");
}
