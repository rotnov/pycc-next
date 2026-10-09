//! Unit tests for the driver's project-module loader (#898, Part 1 of
//! #881, D-222): source-root discovery, dotted-name probing, package
//! `__init__.py` ordering, cycle detection, and every filesystem failure
//! the walk can hit.
//!
//! Every fixture lives inside a [`ScratchDir`] and is addressed by an
//! absolute path, so the tests never depend on (or mutate) the process
//! working directory and can run in parallel. The bare-file-name entry
//! spelling, which is the one case that *is* working-directory-relative,
//! is covered by `tests/issue_881_project_imports.rs` through the real
//! CLI.

use super::*;
use pycc_scratch::ScratchDir;

/// Writes `contents` to `root/relative`, creating parent directories.
fn write(root: &Path, relative: &str, contents: &str) -> PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("a fixture path has a parent"))
        .expect("fixture directories must be creatable");
    std::fs::write(&path, contents).expect("a fixture file must be writable");
    path
}

/// The display paths of a successfully loaded program, in load order.
fn loaded_paths(entry: &Path) -> Vec<String> {
    load(entry, None)
        .unwrap_or_else(|failure| panic!("must load: {}", describe(&failure)))
        .modules
        .into_iter()
        .map(|module| {
            Path::new(&module.display_path)
                .file_name()
                .expect("a loaded module has a file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn describe(failure: &FrontendFailure) -> String {
    match failure {
        FrontendFailure::Input { path, message } => format!("{path}: {message}"),
        FrontendFailure::Compile { files } => files
            .iter()
            .flat_map(|file| {
                file.diagnostics
                    .iter()
                    .map(move |d| format!("{}: {} {}", file.path, d.code, d.message))
            })
            .collect::<Vec<_>>()
            .join("; "),
    }
}

/// The one diagnostic a failing load reports, with the file it belongs to.
fn first_diagnostic(entry: &Path) -> (String, String, String) {
    match load(entry, None)
        .map(|_| ())
        .expect_err("this fixture must fail")
    {
        FrontendFailure::Compile { files } => {
            let file = &files[0];
            let diagnostic = &file.diagnostics[0];
            (
                file.path.clone(),
                diagnostic.code.to_string(),
                diagnostic.message.clone(),
            )
        }
        FrontendFailure::Input { path, message } => {
            panic!("expected a diagnostic: {path}: {message}")
        }
    }
}

fn input_failure(entry: &Path) -> (String, String) {
    match load(entry, None)
        .map(|_| ())
        .expect_err("this fixture must fail")
    {
        FrontendFailure::Input { path, message } => (path, message),
        FrontendFailure::Compile { files } => {
            panic!(
                "expected an input failure, got diagnostics in {}",
                files[0].path
            )
        }
    }
}

const HELPER: &str = "def helper(x: int) -> int:\n    return x + 1\n";
const USES_HELPER: &str =
    "from helper import helper\n\n\ndef main() -> None:\n    print(helper(1))\n";

#[test]
fn a_single_file_with_no_project_import_loads_alone() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    let entry = write(&scratch, "main.py", "def main() -> None:\n    print(1)\n");
    assert_eq!(loaded_paths(&entry), vec!["main.py"]);
}

#[test]
fn a_sibling_module_resolves_against_the_working_directory_root() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "helper.py", HELPER);
    let entry = write(&scratch, "main.py", USES_HELPER);
    assert_eq!(loaded_paths(&entry), vec!["helper.py", "main.py"]);
}

#[test]
fn the_root_walks_out_of_every_enclosing_package() {
    // `pkg/` has an `__init__.py`, so the entry's own directory is inside a
    // package and the absolute name `helper` resolves one level up.
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "helper.py", HELPER);
    write(&scratch, "pkg/__init__.py", "");
    let entry = write(&scratch, "pkg/main.py", USES_HELPER);
    assert_eq!(loaded_paths(&entry), vec!["helper.py", "main.py"]);
}

#[test]
fn a_pycc_toml_entry_names_the_source_root() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(
        &scratch,
        "pycc.toml",
        "[project]\nname = \"demo\"\nentry = \"src/main.py\"\npython = \"3.14\"\n",
    );
    write(&scratch, "src/helper.py", HELPER);
    // `src/` has no `__init__.py`, so only the manifest can put the root
    // there; without it the walk would stop at `src/` anyway, so the
    // dependency is placed one level deeper to make the difference visible.
    write(&scratch, "src/pkg/__init__.py", "");
    let entry = write(&scratch, "src/pkg/main.py", USES_HELPER);
    assert_eq!(loaded_paths(&entry), vec!["helper.py", "main.py"]);
}

#[test]
fn an_unparseable_pycc_toml_is_an_input_failure() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "pycc.toml", "this is not toml\n");
    let entry = write(&scratch, "main.py", USES_HELPER);
    let (path, message) = input_failure(&entry);
    assert!(path.ends_with("pycc.toml"), "unexpected path: {path}");
    assert!(!message.is_empty());
}

#[test]
fn a_pycc_toml_naming_an_unreachable_entry_falls_back_to_the_package_walk() {
    // The manifest's `entry` directory is not an ancestor of the file being
    // compiled, so `climbs_between` refuses it and the package walk decides
    // the root instead.
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(
        &scratch,
        "pycc.toml",
        "[project]\nname = \"demo\"\nentry = \"elsewhere/main.py\"\npython = \"3.14\"\n",
    );
    write(&scratch, "elsewhere/main.py", "");
    write(&scratch, "helper.py", HELPER);
    let entry = write(&scratch, "main.py", USES_HELPER);
    assert_eq!(loaded_paths(&entry), vec!["helper.py", "main.py"]);
}

#[test]
fn a_pycc_toml_naming_a_missing_entry_directory_falls_back_to_the_package_walk() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(
        &scratch,
        "pycc.toml",
        "[project]\nname = \"demo\"\nentry = \"gone/main.py\"\npython = \"3.14\"\n",
    );
    write(&scratch, "helper.py", HELPER);
    let entry = write(&scratch, "main.py", USES_HELPER);
    assert_eq!(loaded_paths(&entry), vec!["helper.py", "main.py"]);
}

#[test]
fn the_standard_library_wins_over_a_same_named_project_module() {
    // `pycc_std` answers `math` before the request ever reaches the loader,
    // so the local `math.py` is not compiled into the program.
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(
        &scratch,
        "math.py",
        "def sqrt(x: float) -> float:\n    return x\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from math import sqrt\n\n\ndef main() -> None:\n    print(sqrt(4.0))\n",
    );
    assert_eq!(loaded_paths(&entry), vec!["main.py"]);
}

#[test]
fn a_module_file_wins_over_a_package_of_the_same_name() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "helper.py", HELPER);
    write(
        &scratch,
        "helper/__init__.py",
        "def helper(x: int) -> str:\n    return \"wrong\"\n",
    );
    let entry = write(&scratch, "main.py", USES_HELPER);
    let program =
        load(&entry, None).unwrap_or_else(|failure| panic!("must load: {}", describe(&failure)));
    assert!(
        program.modules[0].display_path.ends_with("helper.py"),
        "unexpected dependency: {}",
        program.modules[0].display_path
    );
}

#[test]
fn a_package_initializer_is_loaded_before_its_submodule() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "pkg/__init__.py", "print(\"init\")\n");
    write(&scratch, "pkg/mod.py", HELPER);
    let entry = write(
        &scratch,
        "main.py",
        "from pkg.mod import helper\n\n\ndef main() -> None:\n    print(helper(1))\n",
    );
    assert_eq!(
        loaded_paths(&entry),
        vec!["__init__.py", "mod.py", "main.py"]
    );
}

#[test]
fn a_package_initializer_importing_its_own_submodule_is_not_a_cycle() {
    // Loading `pkg.mod` would normally preload `pkg/__init__.py` first, but
    // it is the module currently being loaded, so the preload is skipped.
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(
        &scratch,
        "pkg/__init__.py",
        "from pkg.mod import helper\n\n\ndef wrapper(x: int) -> int:\n    return helper(x)\n",
    );
    write(&scratch, "pkg/mod.py", HELPER);
    let entry = write(
        &scratch,
        "main.py",
        "from pkg import wrapper\n\n\ndef main() -> None:\n    print(wrapper(1))\n",
    );
    assert_eq!(
        loaded_paths(&entry),
        vec!["mod.py", "__init__.py", "main.py"]
    );
}

#[test]
fn an_intermediate_directory_without_an_initializer_still_resolves() {
    // PEP 420: `outer/` is a namespace directory on the way to `outer.mod`,
    // which is a regular module and therefore supported.
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "outer/mod.py", HELPER);
    let entry = write(
        &scratch,
        "main.py",
        "from outer.mod import helper\n\n\ndef main() -> None:\n    print(helper(1))\n",
    );
    assert_eq!(loaded_paths(&entry), vec!["mod.py", "main.py"]);
}

#[test]
fn a_namespace_package_as_the_import_target_is_rejected() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "outer/mod.py", HELPER);
    let entry = write(
        &scratch,
        "main.py",
        "from outer import mod\n\n\ndef main() -> None:\n    print(1)\n",
    );
    let (path, code, message) = first_diagnostic(&entry);
    assert!(path.ends_with("main.py"), "unexpected path: {path}");
    assert_eq!(code, "C0001");
    assert!(
        message.contains("namespace package") && message.contains("is not supported yet"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_bare_import_of_a_real_project_module_is_recognized_but_unsupported() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "helper.py", HELPER);
    let entry = write(
        &scratch,
        "main.py",
        "import helper\n\n\ndef main() -> None:\n    print(1)\n",
    );
    let (_, code, message) = first_diagnostic(&entry);
    assert_eq!(code, "C0001");
    assert!(
        message.contains("module namespace bindings (`import helper`)"),
        "unexpected message: {message}"
    );
}

/// #1381 (Part 3 of #1138): a plain dotted import whose root is neither a
/// project package nor a project module is answered as a CPython module, at
/// top level and nested in a module-level `try`. Unaliased, it binds the
/// root; aliased, the leaf.
#[test]
fn a_plain_dotted_import_that_resolves_nowhere_is_a_foreign_import() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    let entry = write(
        &scratch,
        "main.py",
        "import no.where
import no.where.deep as deep
try:
    import other.place
except ImportError:
    pass


def main() -> None:
    print(1)
",
    );
    let program =
        load(&entry, None).unwrap_or_else(|failure| panic!("must load: {}", describe(&failure)));
    assert_eq!(program.modules.len(), 1, "nothing but the entry loads");
    assert_eq!(
        entry_foreign_plain_imports(&program),
        vec![
            ("no".to_string(), "no.where".to_string()),
            ("deep".to_string(), "no.where.deep".to_string()),
            ("other".to_string(), "other.place".to_string()),
        ]
    );
}

/// #1381: a plain dotted import under a project module or a project package
/// keeps the single-file `C0001`, like the from form: the root is compiled
/// into the artifact, so a host lookup would name a different module.
#[test]
fn a_plain_dotted_import_under_a_project_root_keeps_the_single_file_diagnostic() {
    for (fixture, import) in [
        ("helper.py", "helper.sub"),
        ("pkg/__init__.py", "pkg.missing"),
    ] {
        let scratch = ScratchDir::new("modules_tests").expect("scratch");
        write(&scratch, fixture, "x = 1\n");
        let entry = write(
            &scratch,
            "main.py",
            &format!("import {import}\n\n\ndef main() -> None:\n    print(1)\n"),
        );
        let (_, code, message) = first_diagnostic(&entry);
        assert_eq!(code, "C0001", "{import}");
        assert!(
            message.contains(&format!("import of module `{import}` is not supported yet")),
            "unexpected message: {message}"
        );
    }
}

/// Part 1 of #1138: a dotted module whose root is neither a project package
/// nor a project module is answered as a CPython module for the from form,
/// so the program loads with the name bound as a foreign object.
#[test]
fn a_dotted_from_import_that_resolves_nowhere_is_a_foreign_import() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    let entry = write(
        &scratch,
        "main.py",
        "from no.where import thing


def main() -> None:
    print(1)
",
    );
    let program =
        load(&entry, None).unwrap_or_else(|failure| panic!("must load: {}", describe(&failure)));
    assert_eq!(program.modules.len(), 1, "nothing but the entry loads");
    assert_eq!(
        entry_foreign_from_imports(&program),
        vec![("no.where".to_string(), "thing".to_string(), 0)]
    );
}

/// Part 1 of #1138: a dotted name under a project *module* (`helper.py`)
/// keeps the `C0001` -- `helper` is compiled into the artifact, so a host
/// lookup of `helper.sub` would name a different object.
#[test]
fn a_dotted_from_import_under_a_project_module_keeps_the_c0001() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "helper.py", "x = 1\n");
    let entry = write(
        &scratch,
        "main.py",
        "from helper.sub import y\n\n\ndef main() -> None:\n    print(1)\n",
    );
    let (_, code, message) = first_diagnostic(&entry);
    assert_eq!(code, "C0001");
    assert!(
        message.contains("import of module `helper.sub` is not supported yet"),
        "unexpected message: {message}"
    );
}

/// Part 1 of #1138: a dotted name under a project *package* whose leaf is
/// missing (the probe misses past the first segment) keeps the `C0001`.
#[test]
fn a_dotted_from_import_under_a_project_package_with_a_missing_leaf_keeps_the_c0001() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "pkg/__init__.py", "");
    let entry = write(
        &scratch,
        "main.py",
        "from pkg.missing import y\n\n\ndef main() -> None:\n    print(1)\n",
    );
    let (_, code, message) = first_diagnostic(&entry);
    assert_eq!(code, "C0001");
    assert!(
        message.contains("import of module `pkg.missing` is not supported yet"),
        "unexpected message: {message}"
    );
}

/// #1278: an undotted module found nowhere in the project is answered as a
/// CPython module for the from form too, so the program loads.
#[test]
fn an_undotted_from_import_that_resolves_nowhere_is_a_foreign_import() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    let entry = write(
        &scratch,
        "main.py",
        "from nowhere import thing\n\n\ndef main() -> None:\n    print(1)\n",
    );
    assert_eq!(loaded_paths(&entry), vec!["main.py"]);
}

#[test]
fn a_relative_import_outside_a_package_is_rejected() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "helper.py", HELPER);
    let entry = write(
        &scratch,
        "main.py",
        "from .helper import helper\n\n\ndef main() -> None:\n    print(helper(1))\n",
    );
    let (_, code, message) = first_diagnostic(&entry);
    assert_eq!(code, "T0021");
    assert!(
        message.contains("attempted relative import with no known parent package")
            && message.contains("has no `__init__.py`"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_sibling_relative_import_inside_a_package_resolves() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "pkg/__init__.py", "");
    write(&scratch, "pkg/helper.py", HELPER);
    let entry = write(
        &scratch,
        "pkg/main.py",
        "from .helper import helper\n\n\ndef main() -> None:\n    print(helper(1))\n",
    );
    assert_eq!(loaded_paths(&entry), vec!["helper.py", "main.py"]);
}

#[test]
fn a_parent_relative_import_climbs_one_package() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "pkg/__init__.py", "");
    write(&scratch, "pkg/helper.py", HELPER);
    write(&scratch, "pkg/inner/__init__.py", "");
    let entry = write(
        &scratch,
        "pkg/inner/main.py",
        "from ..helper import helper\n\n\ndef main() -> None:\n    print(helper(1))\n",
    );
    assert_eq!(loaded_paths(&entry), vec!["helper.py", "main.py"]);
}

#[test]
fn a_relative_import_beyond_the_top_level_package_is_rejected() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "pkg/__init__.py", "");
    let entry = write(
        &scratch,
        "pkg/main.py",
        "from ..helper import helper\n\n\ndef main() -> None:\n    print(helper(1))\n",
    );
    let (_, code, message) = first_diagnostic(&entry);
    assert_eq!(code, "T0021");
    assert_eq!(
        message,
        "attempted relative import beyond the top-level package"
    );
}

#[test]
fn a_relative_name_that_resolves_nowhere_is_rejected() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "pkg/__init__.py", "");
    let entry = write(
        &scratch,
        "pkg/main.py",
        "from .missing import helper\n\n\ndef main() -> None:\n    print(helper(1))\n",
    );
    let (_, code, message) = first_diagnostic(&entry);
    assert_eq!(code, "T0021");
    assert!(
        message.contains("no module named `.missing` in `"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_bare_relative_import_binds_the_package_itself() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "pkg/__init__.py", HELPER);
    let entry = write(
        &scratch,
        "pkg/main.py",
        "from . import helper\n\n\ndef main() -> None:\n    print(helper(1))\n",
    );
    assert_eq!(loaded_paths(&entry), vec!["__init__.py", "main.py"]);
}

#[test]
fn an_import_cycle_is_reported_with_its_chain() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(
        &scratch,
        "a.py",
        "from b import second\n\n\ndef first(x: int) -> int:\n    return second(x)\n",
    );
    write(
        &scratch,
        "b.py",
        "from a import first\n\n\ndef second(x: int) -> int:\n    return first(x)\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from a import first\n\n\ndef main() -> None:\n    print(first(1))\n",
    );
    let (path, code, message) = first_diagnostic(&entry);
    assert!(
        path.ends_with("b.py"),
        "reported in the importing file: {path}"
    );
    assert_eq!(code, "E0108");
    assert!(
        message.starts_with("import cycle: ") && message.matches("a.py").count() == 2,
        "unexpected message: {message}"
    );
}

#[test]
fn one_file_reached_by_two_spellings_is_one_module() {
    // `pkg` is a symlink to `real`, so `pkg.mod` and `real.mod` canonicalize
    // to the same file and must be loaded once.
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "real/__init__.py", "");
    write(
        &scratch,
        "real/mod.py",
        "def helper(x: int) -> int:\n    return x + 1\n\n\ndef other(x: int) -> int:\n    return x - 1\n",
    );
    symlink_dir(&scratch.join("real"), &scratch.join("pkg"));
    let entry = write(
        &scratch,
        "main.py",
        "from real.mod import helper\nfrom pkg.mod import other\n\n\n\
         def main() -> None:\n    print(helper(1) + other(1))\n",
    );
    let program =
        load(&entry, None).unwrap_or_else(|failure| panic!("must load: {}", describe(&failure)));
    let modules: Vec<&str> = program
        .modules
        .iter()
        .map(|module| module.display_path.as_str())
        .collect();
    assert_eq!(
        modules.len(),
        3,
        "two __init__ spellings plus mod.py and main.py would be four: {modules:?}"
    );
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("a fixture symlink must be creatable");
}

#[cfg(not(unix))]
fn symlink_dir(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_dir(target, link).expect("a fixture symlink must be creatable");
}

#[test]
fn an_undecodable_dependency_is_an_input_failure() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    std::fs::write(scratch.join("helper.py"), [0xff, 0xfe, 0x00]).expect("write");
    let entry = write(&scratch, "main.py", USES_HELPER);
    let (path, message) = input_failure(&entry);
    assert!(path.ends_with("helper.py"), "unexpected path: {path}");
    assert!(!message.is_empty());
}

#[test]
fn a_dependency_that_does_not_parse_reports_against_its_own_file() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "helper.py", "def helper(\n");
    let entry = write(&scratch, "main.py", USES_HELPER);
    let (path, _, _) = first_diagnostic(&entry);
    assert!(path.ends_with("helper.py"), "unexpected path: {path}");
}

#[test]
fn a_dependency_that_does_not_lower_reports_against_its_own_file() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
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
    let (path, code, _) = first_diagnostic(&entry);
    assert!(path.ends_with("helper.py"), "unexpected path: {path}");
    assert_eq!(code, "C0001");
}

#[test]
fn a_missing_entry_file_is_an_input_failure() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    let entry = scratch.join("missing.py");
    let (path, message) = input_failure(&entry);
    assert!(path.ends_with("missing.py"), "unexpected path: {path}");
    assert!(!message.is_empty());
}

#[cfg(unix)]
#[test]
fn an_unreadable_dependency_is_an_input_failure() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    let helper = write(&scratch, "helper.py", HELPER);
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    let entry = write(&scratch, "main.py", USES_HELPER);
    let result = load(&entry, None);
    // Running as root defeats the permission bit entirely; the load then
    // simply succeeds and there is nothing to assert.
    if let Err(FrontendFailure::Input { path, message }) = result {
        assert!(path.ends_with("helper.py"), "unexpected path: {path}");
        assert!(!message.is_empty());
    }
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o644)).expect("chmod");
}

#[cfg(unix)]
#[test]
fn an_unreadable_pycc_toml_is_an_input_failure() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    let toml = write(
        &scratch,
        "pycc.toml",
        "[project]\nname = \"demo\"\nentry = \"main.py\"\npython = \"3.14\"\n",
    );
    std::fs::set_permissions(&toml, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    write(&scratch, "helper.py", HELPER);
    let entry = write(&scratch, "main.py", USES_HELPER);
    let result = load(&entry, None);
    if let Err(FrontendFailure::Input { path, message }) = result {
        assert!(path.ends_with("pycc.toml"), "unexpected path: {path}");
        assert!(!message.is_empty());
    }
    std::fs::set_permissions(&toml, std::fs::Permissions::from_mode(0o644)).expect("chmod");
}

#[test]
fn submodule_names_lists_modules_and_packages_only() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "pkg/__init__.py", "");
    write(&scratch, "pkg/beta.py", "");
    write(&scratch, "pkg/alpha/__init__.py", "");
    write(&scratch, "pkg/README.txt", "");
    assert_eq!(
        submodule_names(&scratch.join("pkg")),
        vec!["alpha".to_string(), "beta".to_string()]
    );
}

#[test]
fn submodule_names_of_a_missing_directory_is_empty() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    assert!(submodule_names(&scratch.join("gone")).is_empty());
}

#[test]
fn climbs_between_counts_components_and_refuses_a_non_ancestor() {
    assert_eq!(
        climbs_between(Path::new("/a/b/c"), Path::new("/a")),
        Some(2)
    );
    assert_eq!(climbs_between(Path::new("/a"), Path::new("/a")), Some(0));
    assert_eq!(climbs_between(Path::new("/a/b"), Path::new("/x")), None);
}

#[test]
fn display_root_trims_components_and_pads_with_parent_links() {
    assert_eq!(display_root(Path::new("a/b/c"), 0), PathBuf::from("a/b/c"));
    assert_eq!(display_root(Path::new("a/b/c"), 2), PathBuf::from("a"));
    // Fewer components than climbs: the entry was named by a bare file
    // name, so the root is above the working directory.
    assert_eq!(display_root(Path::new(""), 2), PathBuf::from("../.."));
    assert_eq!(display_root(Path::new("a"), 3), PathBuf::from("../.."));
}

#[test]
fn a_failing_package_initializer_is_reported_against_its_own_file() {
    // The `__init__.py` preloaded on the way to `pkg.mod` is a module like
    // any other: its own failure aborts the walk and renders against it.
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "pkg/__init__.py", "def broken(:\n");
    write(&scratch, "pkg/mod.py", HELPER);
    let entry = write(
        &scratch,
        "main.py",
        "from pkg.mod import helper\n\n\ndef main() -> None:\n    print(helper(1))\n",
    );
    let (path, _, _) = first_diagnostic(&entry);
    assert!(path.ends_with("__init__.py"), "unexpected file: {path}");
}

#[test]
fn a_wildcard_import_of_a_project_module_is_rejected() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "helper.py", HELPER);
    let entry = write(&scratch, "main.py", "from helper import *\n");
    let (path, code, message) = first_diagnostic(&entry);
    assert!(path.ends_with("main.py"), "unexpected file: {path}");
    assert_eq!(code, "C0001");
    assert!(
        message.contains("wildcard import"),
        "unexpected message: {message}"
    );
}

#[test]
fn aliasing_a_name_imported_from_a_project_module_is_rejected() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "helper.py", HELPER);
    let entry = write(&scratch, "main.py", "from helper import helper as h\n");
    let (path, code, message) = first_diagnostic(&entry);
    assert!(path.ends_with("main.py"), "unexpected file: {path}");
    assert_eq!(code, "C0001");
    assert!(
        message.contains("aliasing is not supported yet"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_type_alias_reached_through_two_modules_lowers_and_binds() {
    // `main` imports `Number` from the module that defines it and again
    // from a module that re-exports it, so the alias reaches `main` twice
    // and the whole program must still load and lower.
    // The dedup guard in `module.rs` that skips the second copy has no
    // observable effect in Part 1 (`strip_imported` removes every
    // imported entry again), so this covers the reachability of the
    // path, not the guard itself.
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(
        &scratch,
        "defs.py",
        "type Number = int\n\n\ndef identity(x: Number) -> Number:\n    return x\n",
    );
    write(
        &scratch,
        "reexport.py",
        "from defs import Number, identity\n\n\ndef twice(x: Number) -> Number:\n    return identity(x) + identity(x)\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from defs import Number\nfrom reexport import Number, twice\n\n\ndef main(x: Number) -> Number:\n    return twice(x)\n",
    );
    assert_eq!(
        loaded_paths(&entry),
        vec!["defs.py", "reexport.py", "main.py"]
    );
}

#[test]
fn identity_path_falls_back_to_the_path_as_given() {
    // Only reachable when the filesystem cannot canonicalize the path at
    // all; the memoization key is then the path itself, and the read that
    // follows reports the real failure.
    let missing = Path::new("/pycc-does-not-exist/nowhere.py");
    assert_eq!(identity_path(missing), missing.to_path_buf());
}

#[test]
fn a_bare_file_name_importer_renders_its_directory_as_a_single_dot() {
    // The entry can be named by a bare file name (`pycc check main.py`),
    // whose directory spelling is empty; a relative import from it must
    // still render a directory a diagnostic can show. Exercised directly
    // because the spelling is working-directory-relative by construction;
    // `tests/issue_881_project_imports.rs` covers the same shape through
    // the real CLI.
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    let importer = write(&scratch, "main.py", "from . import helper\n");
    let mut loader = Loader {
        modules: Vec::new(),
        memo: HashMap::new(),
        in_progress: Vec::new(),
        entry_dir: PathBuf::new(),
        entry_display_dir: PathBuf::new(),
        root: None,
        manifest: None,
        entry_module_name: None,
        relative_imports: RelativeImports::Project,
        entry_package: None,
        ext_module: false,
    };
    let request = ProjectImportRequest {
        level: 1,
        module: Some("helper".to_string()),
        names: vec!["helper".to_string()],
        span: pycc_diag::Span::new(0, 0),
        nested: false,
    };
    let base = loader
        .base_dir(&request, "main.py", &importer)
        .unwrap_or_else(|failure| panic!("base resolution must not fail: {}", describe(&failure)));
    let rejection = base
        .rejection
        .expect("the scratch directory is not a package, so the base is rejected");
    assert!(
        rejection.contains("`.` has no `__init__.py`"),
        "unexpected rejection: {rejection}"
    );
}

/// Loads `entry` under `pycc build --ext --foreign-relative-imports`'s mode
/// (#1366).
fn load_foreign(entry: &Path) -> Result<LoadedProgram, FrontendFailure> {
    load_with(entry, None, RelativeImports::ForeignFromEntry, true)
}

/// `(local_name, module_path)` of every plain foreign import of the entry
/// module, in source order (#1381).
fn entry_foreign_plain_imports(program: &LoadedProgram) -> Vec<(String, String)> {
    program
        .modules
        .last()
        .expect("the entry is loaded last")
        .module
        .hir
        .imports
        .iter()
        .filter_map(|binding| match binding {
            pycc_hir::ImportBinding::Foreign {
                local_name,
                module_path,
                from: None,
                ..
            } => Some((local_name.clone(), module_path.clone())),
            _ => None,
        })
        .collect()
}

/// The `(module_path, name, level)` of every foreign from-import binding of
/// the loaded program's entry module.
fn entry_foreign_from_imports(program: &LoadedProgram) -> Vec<(String, String, u32)> {
    program
        .modules
        .last()
        .expect("the entry is loaded last")
        .module
        .hir
        .imports
        .iter()
        .filter_map(|binding| match binding {
            pycc_hir::ImportBinding::Foreign {
                module_path,
                from: Some(from),
                ..
            } => Some((module_path.clone(), from.name.clone(), from.level)),
            _ => None,
        })
        .collect()
}

/// #1366: under `ForeignFromEntry` the entry's relative import is foreign
/// without any filesystem probe: a same-named sibling file is never loaded,
/// and no `__init__.py` is needed. The dots travel as `level`, never in
/// `module_path`, which is `""` for `from . import x`.
#[test]
fn the_entry_relative_imports_are_foreign_under_foreign_from_entry() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "sib.py", "x = 1\n");
    let entry = write(
        &scratch,
        "main.py",
        "from .sib import x\nfrom . import other\nfrom ..a.b import c\n\n\n\
         def main() -> None:\n    print(1)\n",
    );
    let program =
        load_foreign(&entry).unwrap_or_else(|failure| panic!("must load: {}", describe(&failure)));
    assert_eq!(program.modules.len(), 1, "no sibling file is loaded");
    assert_eq!(
        entry_foreign_from_imports(&program),
        vec![
            ("sib".to_string(), "x".to_string(), 1),
            (String::new(), "other".to_string(), 1),
            ("a.b".to_string(), "c".to_string(), 2),
        ]
    );
}

/// #1366: the opt-in is the entry module's alone. A dependency's relative
/// import stays a project import (D-222), both when it resolves and when it
/// is `T0021` outside a package.
#[test]
fn a_dependency_relative_import_stays_a_project_import_under_foreign_from_entry() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "pkg/__init__.py", "");
    write(&scratch, "pkg/inner.py", HELPER);
    write(
        &scratch,
        "pkg/helper.py",
        "from .inner import helper\n\n\ndef twice(x: int) -> int:\n    return helper(x) * 2\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from pkg.helper import twice\n\n\ndef main() -> None:\n    print(twice(1))\n",
    );
    let names: Vec<String> = load_foreign(&entry)
        .unwrap_or_else(|failure| panic!("must load: {}", describe(&failure)))
        .modules
        .iter()
        .map(|module| {
            Path::new(&module.display_path)
                .file_name()
                .expect("a loaded module has a file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        names,
        vec!["__init__.py", "inner.py", "helper.py", "main.py"]
    );

    let outside = ScratchDir::new("modules_tests").expect("scratch");
    write(&outside, "helper.py", "from .nowhere import thing\n");
    let entry = write(
        &outside,
        "main.py",
        "from helper import thing\n\n\ndef main() -> None:\n    print(1)\n",
    );
    let failure = load_foreign(&entry)
        .map(|_| ())
        .expect_err("the dependency's relative import is outside a package");
    let rendered = describe(&failure);
    assert!(
        rendered.contains("helper.py: T0021")
            && rendered.contains("attempted relative import with no known parent package"),
        "{rendered}"
    );
}

mod entry_package;
mod nested;
