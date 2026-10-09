//! Issue #1425: a native module that imports a CPython object from a
//! sibling project module calls `append`, `pop`, `get` and `add` on it as
//! the foreign method call. Since #1095 a module admits object receivers once
//! it can hold an object; since #1425 that includes a module whose direct
//! project dependency (or a package `__init__` on its path) can hold one.
//! The dependency publishes its final state (`LoweredModule::object_receivers`)
//! and the driver ORs it into each importer, so the bit is transitive and
//! covers the importer's whole body.
//!
//! The refusals and `pycc check` results run everywhere. Diagnostics in a
//! multi-module check render at `<file>:1:1` (#1088), so they are matched by
//! message text. The embedded-executable comparison with CPython 3.14.7 is
//! `#[ignore]`d for the reason every embed test is.
//!
//! [#1425]: https://github.com/rotnov/pycc/issues/1425

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn rendered(output: &Output) -> String {
    let mut text = stdout_of(output);
    text.push_str(&stderr_of(output));
    text
}

fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create the package directory");
    }
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

/// Writes `files` into a fresh scratch directory and runs `pycc check` on
/// its `m.py`.
fn check(tag: &str, files: &[(&str, &str)], entry: &str) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    for (file, body) in files {
        write(&dir, file, body);
    }
    pycc()
        .arg("check")
        .arg(write(&dir, "m.py", entry))
        .output()
        .expect("pycc should spawn")
}

fn assert_passes(tag: &str, files: &[(&str, &str)], entry: &str) {
    let output = check(tag, files, entry);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{entry}: {}",
        rendered(&output)
    );
}

/// The objects live two modules away: `dep1` binds them, `dep2` re-exports
/// them, and the entry module imports them from `dep2`.
const DEP1: &str = "import collections\nimport gc\n\nd = collections.deque()\nod = \
                    collections.OrderedDict()\ng = gc\n";
const DEP2: &str = "from dep1 import d, od, g\n";

fn transitive() -> [(&'static str, &'static str); 2] {
    [("dep1.py", DEP1), ("dep2.py", DEP2)]
}

/// The issue's shape, re-exported once: every container name on an object
/// imported through a project module passes `pycc check`.
#[test]
fn an_object_imported_through_a_re_export_takes_the_foreign_method_call() {
    assert_passes(
        "obj_1425_transitive",
        &transitive(),
        "from dep2 import d, od, g\n\nd.append(2)\nd.pop()\nod.get(\"a\")\ng.garbage.append(1)\n",
    );
}

/// A package `__init__` re-exporting the object on the import path. `sub`
/// imports `pkg` itself, so its own published bit already carries the
/// admission: this shape executes the driver's `package_inits` OR (kept for
/// parity with #1188) without being able to tell it apart, since the entry
/// module cannot name an init's object without importing it.
#[test]
fn an_object_reached_through_a_package_init_passes_check() {
    assert_passes(
        "obj_1425_package",
        &[
            ("dep1.py", DEP1),
            ("pkg/__init__.py", "from dep1 import g\n"),
            ("pkg/sub.py", "from pkg import g\n"),
        ],
        "from pkg.sub import g\n\ng.garbage.append(1)\n",
    );
}

/// The call positions an importer can reach the object from: a module-level
/// function, a class method, and an `isinstance`-narrowed branch.
#[test]
fn every_position_in_the_importer_passes_check() {
    for (tag, entry) in [
        (
            "obj_1425_def",
            "from dep2 import d\n\n\ndef push() -> None:\n    d.append(3)\n\n\npush()\n",
        ),
        (
            "obj_1425_method",
            "from dep2 import od\n\n\nclass Box:\n    def look(self) -> None:\n        \
             od.get(\"a\")\n\n\nBox().look()\n",
        ),
        (
            "obj_1425_isinstance",
            "from dep2 import d\n\nif isinstance(d, list):\n    d.append(1)\n",
        ),
    ] {
        assert_passes(tag, &transitive(), entry);
    }
}

/// Inherited admission covers the importer's whole body, so a function
/// defined above the import that binds the object passes too -- as it does
/// above a module's own foreign import since #1482.
#[test]
fn inherited_admission_covers_code_above_the_import() {
    assert_passes(
        "obj_1425_above",
        &[("dep.py", "import gc\n\ng = gc\n")],
        "def f() -> None:\n    g.garbage.append(1)\n\n\nfrom dep import g\n\nf()\n",
    );
}

/// Asserts `entry`, checked beside `files`, fails with exactly one error
/// whose message contains `needle`.
fn assert_one_error(tag: &str, files: &[(&str, &str)], entry: &str, code: &str, needle: &str) {
    let output = check(tag, files, entry);
    assert_eq!(output.status.code(), Some(1), "{}", rendered(&output));
    let text = rendered(&output);
    assert_eq!(text.matches("error[").count(), 1, "{text}");
    assert!(text.contains(&format!("error[{code}]")), "{text}");
    assert!(text.contains(needle), "{text}");
}

/// A function-local list that shadows the imported name keeps its native
/// container reading, so a wrong element type is still `T0021`.
#[test]
fn a_local_list_shadowing_the_import_keeps_its_diagnostic() {
    assert_one_error(
        "obj_1425_shadow",
        &[("dep.py", "import gc\n\ng = gc\n")],
        "from dep import g\n\n\ndef f() -> None:\n    g = [1]\n    g.append(\"s\")\n\n\nf()\n",
        "T0021",
        "cannot append `str` to a list of `int`",
    );
}

/// A native list, set and dict beside a sibling import keep their native
/// diagnostics, exactly as beside a module's own `import gc`.
#[test]
fn native_containers_beside_a_sibling_import_keep_their_diagnostics() {
    let dep = [("dep.py", "import gc\n\ng = gc\n")];
    assert_one_error(
        "obj_1425_list",
        &dep,
        "from dep import g\n\nxs = [1]\nxs.append(\"s\")\n",
        "T0021",
        "cannot append `str` to a list of `int`",
    );
    assert_one_error(
        "obj_1425_set",
        &dep,
        "from dep import g\n\nss = {1}\nss.add(\"x\")\n",
        "T0021",
        "cannot add `str` to a set of `int`",
    );
    // The one-argument `get` refusal is byte for byte the one a module with
    // its own foreign import gets.
    let sibling = check(
        "obj_1425_get_sibling",
        &dep,
        "from dep import g\n\nds = {\"a\": 1}\nds.get(\"a\")\n",
    );
    let own = check(
        "obj_1425_get_own",
        &[],
        "import gc\n\nds = {\"a\": 1}\nds.get(\"a\")\n",
    );
    assert_eq!(sibling.status.code(), Some(1), "{}", rendered(&sibling));
    let message = |output: &Output| {
        let text = rendered(output);
        text.lines()
            .find(|line| line.starts_with("error[C0001]"))
            .unwrap_or_else(|| panic!("no C0001 in {text}"))
            .to_string()
    };
    assert!(
        message(&sibling).contains(".get()"),
        "{}",
        message(&sibling)
    );
    assert_eq!(message(&sibling), message(&own));
}

/// The embedded program: deque `append`/`pop`, OrderedDict `get`,
/// `gc.garbage.append`/`pop`, the def and method positions, the
/// `isinstance` branch, and a native list observed through `len` (printing
/// a native list is the separate #1220).
#[cfg(not(windows))]
const EMBEDDED: &str = "from dep2 import d, od, g\n\n\ndef push(n: int) -> None:\n    \
                        d.append(n)\n\n\nclass Box:\n    def look(self) -> None:\n        \
                        print(od.get(\"a\"))\n\n\nd.append(2)\npush(3)\nprint(len(d))\n\
                        print(d.pop())\nBox().look()\ng.garbage.append(5)\n\
                        print(g.garbage.pop())\nif isinstance(d, list):\n    d.append(1)\n\
                        xs = [1, 2]\nxs.append(3)\nprint(len(xs))\n";

#[cfg(not(windows))]
fn build_embedded(dir: &Path) -> PathBuf {
    write(dir, "dep1.py", DEP1);
    write(dir, "dep2.py", DEP2);
    let output = pycc()
        .arg("build")
        .arg(write(dir, "m.py", EMBEDDED))
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(output.status.success(), "{}", rendered(&output));
    let marker = std::fs::read_to_string(dir.join("app.pycc").join("PYCC-BUNDLE"))
        .expect("an embedded build writes its marker");
    let mut lines = marker.lines();
    assert_eq!(lines.next(), Some("pycc-bundle 1"));
    assert_eq!(lines.next(), Some("python 3.14.7"), "{marker}");
    let executable = lines
        .next()
        .and_then(|line| line.strip_prefix("executable "))
        .expect("the marker names its interpreter");
    PathBuf::from(executable)
}

#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn sibling_objects_match_cpython_3_14_7_in_an_embedded_executable() {
    let dir = ScratchDir::new("embed_1425_sibling").expect("scratch");
    let python = build_embedded(&dir);
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
    let oracle = Command::new(python)
        .arg(dir.join("m.py"))
        .output()
        .expect("CPython runs the oracle program");
    assert_eq!(stdout_of(&oracle), "2\n3\nNone\n5\n3\n");
    assert_eq!(embedded.stdout, oracle.stdout);
}
