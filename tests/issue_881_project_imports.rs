//! End-to-end coverage for project imports (#898, Part 1 of #881, D-222):
//! the CLI resolves an entry file's project imports, links every reachable
//! module into one program, and renders each diagnostic against the file
//! that actually owns it.
//!
//! Every fixture is a real directory tree under a `ScratchDir` driven
//! through the real `pycc` binary, because the file layout *is* the
//! feature under test.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::Command;

fn pycc_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write(root: &Path, relative: &str, contents: &str) -> PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("a fixture path has a parent")).unwrap();
    std::fs::write(&path, contents).unwrap();
    path
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn both(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

fn run(args: &[&str], cwd: Option<&Path>) -> Run {
    let mut command = Command::new(pycc_bin());
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().expect("the pycc binary must run");
    Run {
        code: output.status.code().expect("pycc must exit normally"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn check(entry: &Path) -> Run {
    run(&["check", entry.to_str().unwrap()], None)
}

/// The two-file reproduction from #881: a helper module and an entry that
/// imports one name from it.
fn repro(root: &Path) -> PathBuf {
    write(
        root,
        "helper.py",
        "def double(x: int) -> int:\n    return x * 2\n",
    );
    write(
        root,
        "main.py",
        "from helper import double\n\nprint(double(21))\n",
    )
}

#[test]
fn the_reproduction_checks_clean() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    let entry = repro(&scratch);
    let result = check(&entry);
    assert_eq!(result.code, 0, "{}", result.both());
    assert!(result.both().trim().is_empty(), "{}", result.both());
}

#[test]
fn the_reproduction_builds_and_runs() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    let entry = repro(&scratch);
    let binary = scratch.join("prog");
    let built = run(
        &[
            "build",
            entry.to_str().unwrap(),
            "-o",
            binary.to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(built.code, 0, "{}", built.both());
    let output = Command::new(&binary)
        .output()
        .expect("the program must run");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
}

#[test]
fn the_reproduction_runs_through_the_run_subcommand() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    let entry = repro(&scratch);
    let result = run(&["run", entry.to_str().unwrap()], None);
    assert_eq!(result.code, 0, "{}", result.both());
    assert!(result.stdout.contains("42"), "{}", result.both());
}

#[test]
fn an_entry_named_by_a_bare_file_name_resolves_its_siblings() {
    // The one working-directory-relative spelling: the entry's directory
    // component is empty, so the loader substitutes `.`.
    let scratch = ScratchDir::new("issue_881").unwrap();
    repro(&scratch);
    let result = run(&["check", "main.py"], Some(&scratch));
    assert_eq!(result.code, 0, "{}", result.both());
}

#[test]
fn a_relative_import_from_a_bare_file_name_entry_names_the_current_directory() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(
        &scratch,
        "helper.py",
        "def double(x: int) -> int:\n    return x * 2\n",
    );
    write(
        &scratch,
        "main.py",
        "from .helper import double\n\nprint(double(21))\n",
    );
    let result = run(&["check", "main.py"], Some(&scratch));
    assert_eq!(result.code, 1, "{}", result.both());
    assert!(
        result
            .both()
            .contains("attempted relative import with no known parent package"),
        "{}",
        result.both()
    );
}

#[test]
fn a_dependency_lowering_gap_is_reported_against_the_dependency() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(
        &scratch,
        "helper.py",
        "async def bare() -> None:\n    pass\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from helper import bare\n\n\ndef main() -> None:\n    print(1)\n",
    );
    let result = check(&entry);
    assert_eq!(result.code, 1, "{}", result.both());
    assert!(result.both().contains("helper.py"), "{}", result.both());
    assert!(!result.both().contains("main.py"), "{}", result.both());
}

#[test]
fn a_dependency_function_type_error_is_reported_against_the_dependency() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(
        &scratch,
        "helper.py",
        "def double(x: int) -> int:\n    return \"nope\"\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from helper import double\n\n\ndef main() -> None:\n    print(double(1))\n",
    );
    let result = check(&entry);
    assert_eq!(result.code, 1, "{}", result.both());
    assert!(result.both().contains("helper.py"), "{}", result.both());
}

#[test]
fn a_dependency_top_level_type_error_is_reported_against_the_dependency() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(
        &scratch,
        "helper.py",
        "def double(x: int) -> int:\n    return x * 2\n\n\nbad: int = \"nope\"\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from helper import double\n\n\ndef main() -> None:\n    print(double(1))\n",
    );
    let result = check(&entry);
    assert_eq!(result.code, 1, "{}", result.both());
    assert!(result.both().contains("helper.py"), "{}", result.both());
}

#[test]
fn a_program_wide_pre_check_failure_is_reported_against_the_entry() {
    // The redefinition spans two files, so no single item owns it; the
    // driver renders it against the entry.
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(
        &scratch,
        "helper.py",
        "def shared(x: int) -> int:\n    return x\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from helper import shared\n\n\ndef main() -> None:\n    print(shared(1))\n",
    );
    // A second, incompatible signature for an imported name is a link-time
    // collision, so the same shape is exercised through the entry's own
    // duplicate instead.
    std::fs::write(
        &entry,
        "def shared(x: int) -> int:\n    return x\n\n\ndef shared(x: str) -> str:\n    return x\n\n\ndef main() -> None:\n    print(shared(1))\n",
    )
    .unwrap();
    let result = check(&entry);
    assert_eq!(result.code, 1, "{}", result.both());
    assert!(result.both().contains("main.py"), "{}", result.both());
}

#[test]
fn a_flat_namespace_collision_names_both_files() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(
        &scratch,
        "helper.py",
        "def double(x: int) -> int:\n    return x * 2\n\n\ndef report() -> None:\n    print(1)\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from helper import double\n\n\ndef report() -> None:\n    print(2)\n\n\nprint(double(21))\n",
    );
    let result = check(&entry);
    assert_eq!(result.code, 1, "{}", result.both());
    assert!(
        result
            .both()
            .contains("top-level name `report` is already defined by")
            && result.both().contains("helper.py")
            && result.both().contains("main.py"),
        "{}",
        result.both()
    );
}

#[test]
fn json_output_carries_the_dependency_path() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(
        &scratch,
        "helper.py",
        "def double(x: int) -> int:\n    return \"nope\"\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from helper import double\n\n\ndef main() -> None:\n    print(double(1))\n",
    );
    let result = run(
        &["check", entry.to_str().unwrap(), "--error-format", "json"],
        None,
    );
    assert_eq!(result.code, 1, "{}", result.both());
    let value: serde_json::Value =
        serde_json::from_str(result.stdout.lines().next().expect("one JSON line")).unwrap();
    assert!(
        value["spans"][0]["file"]
            .as_str()
            .expect("the JSON payload names a file")
            .ends_with("helper.py"),
        "{}",
        result.stdout
    );
}

#[test]
fn checking_two_paths_reports_each_program_once() {
    // `b.py` imports `a.py`, so `a`'s own diagnostic appears while checking
    // `a` and again while checking `b` -- the two runs are independent
    // programs, and the combined output is exactly their concatenation.
    let scratch = ScratchDir::new("issue_881").unwrap();
    let a = write(&scratch, "a.py", "async def bare() -> None:\n    pass\n");
    let b = write(
        &scratch,
        "b.py",
        "from a import bare\n\n\ndef main() -> None:\n    print(1)\n",
    );
    let only_a = check(&a);
    let only_b = check(&b);
    let both = run(&["check", a.to_str().unwrap(), b.to_str().unwrap()], None);
    assert_eq!(both.code, 1, "{}", both.both());
    assert_eq!(both.both(), format!("{}{}", only_a.both(), only_b.both()));
}

#[test]
fn a_package_initializer_runs_before_the_module_that_needs_it() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(&scratch, "pkg/__init__.py", "print(\"init\")\n");
    write(
        &scratch,
        "pkg/mod.py",
        "print(\"mod\")\n\n\ndef double(x: int) -> int:\n    return x * 2\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from pkg.mod import double\n\nprint(double(21))\n",
    );
    let result = run(&["run", entry.to_str().unwrap()], None);
    assert_eq!(result.code, 0, "{}", result.both());
    let lines: Vec<&str> = result.stdout.lines().collect();
    assert_eq!(lines, vec!["init", "mod", "42"], "{}", result.stdout);
}

#[test]
fn a_namespace_directory_on_the_way_to_a_module_is_allowed() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(
        &scratch,
        "outer/mod.py",
        "def double(x: int) -> int:\n    return x * 2\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from outer.mod import double\n\nprint(double(21))\n",
    );
    let result = check(&entry);
    assert_eq!(result.code, 0, "{}", result.both());
}

#[test]
fn a_build_failure_in_a_dependency_reaches_stderr() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(
        &scratch,
        "helper.py",
        "async def bare() -> None:\n    pass\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from helper import bare\n\n\ndef main() -> None:\n    print(1)\n",
    );
    let binary = scratch.join("prog");
    let result = run(
        &[
            "build",
            entry.to_str().unwrap(),
            "-o",
            binary.to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(result.code, 1, "{}", result.both());
    assert!(result.stderr.contains("helper.py"), "{}", result.stderr);
    assert!(!binary.exists(), "a failing build must not leave a binary");
}

#[test]
fn a_missing_entry_file_is_still_an_input_error() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    let result = check(&scratch.join("gone.py"));
    assert_eq!(result.code, 2, "{}", result.both());
    assert!(result.stderr.contains("gone.py"), "{}", result.stderr);
}

#[test]
fn an_unparseable_manifest_is_an_input_error() {
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(&scratch, "pycc.toml", "not a manifest\n");
    let entry = repro(&scratch);
    let result = check(&entry);
    assert_eq!(result.code, 2, "{}", result.both());
    assert!(result.stderr.contains("pycc.toml"), "{}", result.stderr);
}

// -- Part 1 of #883 (#962): stdlib module aliases across a linked program --

#[test]
fn a_stdlib_alias_in_a_dependency_is_visible_to_the_whole_linked_program() {
    // `pycc_hir::program::link` concatenates every module's import table
    // into one flat `HirModule`, so the alias table `pycc_types` builds is
    // program-wide: `a.py`'s `import math as m` makes a parameter named
    // `m` around a *canonical* `math.sqrt` call in `main.py` a false
    // reject. This pins that fail-closed residual (recorded in the #962
    // ADR) until #901's per-module namespaces close it.
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(
        &scratch,
        "a.py",
        "import math as m\n\n\ndef root(x: float) -> float:\n    return m.sqrt(x)\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "import math\nfrom a import root\n\n\ndef f(m: float) -> float:\n    return math.sqrt(m) + root(m)\n",
    );
    let result = check(&entry);
    assert_eq!(result.code, 1, "{}", result.both());
    assert!(result.both().contains("C0001"), "{}", result.both());
    assert!(
        result.both().contains(
            "`m` is a local name here, not the stdlib `math` module -- attribute access on a \
             non-module value is not supported yet"
        ),
        "{}",
        result.both()
    );
    assert!(result.both().contains("main.py"), "{}", result.both());
}

#[test]
fn a_stdlib_alias_in_a_dependency_does_not_disturb_an_unshadowed_entry() {
    // The same two-module shape without a colliding local: the alias in
    // `a.py` and the canonical spelling in `main.py` both resolve, and the
    // program checks clean.
    let scratch = ScratchDir::new("issue_881").unwrap();
    write(
        &scratch,
        "a.py",
        "import math as m\n\n\ndef root(x: float) -> float:\n    return m.sqrt(x)\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "import math\nfrom a import root\n\n\ndef f(x: float) -> float:\n    return math.sqrt(x) + root(x)\n\n\nprint(f(16.0))\n",
    );
    let result = check(&entry);
    assert_eq!(result.code, 0, "{}", result.both());
}
