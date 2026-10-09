//! #1199: an `--ext` module binds each export as its definition executes,
//! as CPython binds a module-level name, instead of publishing every
//! export before the body runs.
//!
//! PEP 489 puts the module in `sys.modules` before its `Py_mod_exec` slot
//! runs, so a foreign module the body imports can import this one back and
//! read it mid-body. Before #1199 every function and class was already an
//! attribute at that point: a name whose `def` had not run yet was visible,
//! and calling it went through a still-null `fnptr_` slot and crashed the
//! host. Each program here is run twice -- built with `--ext`, and imported
//! as the same source under CPython -- and the outputs are compared, with
//! `str(e).replace(m.__file__, "<file>")` normalising the one place the
//! artifact's path appears. The foreign module lives in its own directory,
//! so pycc never links it as a project module, and both runs use a working
//! directory that is on neither side's `PYTHONPATH` (a cwd holding the
//! module changes CPython's circular-import wording).
//!
//! Four cases are not compared with CPython, each pinning a residual
//! `docs/RUNTIME.md` records.
//! [`an_instance_escaping_before_its_class_statement_raises_name_error`]:
//! the null guard every generated wrapper now opens with turns what
//! CPython reports inside the constructor's caller into a catchable
//! `NameError` at the method call (the construction itself is #1490).
//! [`a_name_redefined_after_the_cycle_is_hidden_until_its_last_definition`]:
//! a redefined function stays absent until its last definition runs,
//! where CPython would show the earlier one.
//! [`a_nested_exec_before_the_class_statement_leaves_each_module_its_own_class`]
//! and
//! [`a_nested_exec_after_the_class_statement_leaves_each_module_its_own_class`]:
//! a nested exec of the same artifact leaves the outer module a class
//! compiled code no longer builds, as before #1199.
//!
//! Every test here is `#[ignore]`d for the reason every `ext` and embedded
//! test is: it builds against and runs an installed CPython with
//! development headers. The generated-text and IR tests that run on the
//! coverage host are `src/ext_build_tests/publication_order.rs` and
//! `crates/pycc_codegen/src/ext_publish_tests.rs`.

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

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

fn subdir(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("create the fixture directory");
    dir
}

fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

/// The fixture layout: the compiled module's source in `src/`, its foreign
/// helper in `lib/`, the extension in `ext/`, and a working directory
/// `run/` that is on neither `PYTHONPATH`.
struct Fixture {
    _root: ScratchDir,
    src: PathBuf,
    lib: PathBuf,
    ext: PathBuf,
    run: PathBuf,
}

impl Fixture {
    /// Writes `module` as `my.py` and `helper` as `cb.py`, and builds the
    /// extension `my`.
    fn new(label: &str, module: &str, helper: &str) -> Self {
        let root = ScratchDir::new(label).expect("scratch");
        let src = subdir(&root, "src");
        let lib = subdir(&root, "lib");
        let ext = subdir(&root, "ext");
        let run = subdir(&root, "run");
        let source = write(&src, "my.py", module);
        write(&lib, "cb.py", helper);
        let build = pycc()
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(ext.join("my"))
            .arg("--ext")
            .output()
            .expect("pycc should spawn");
        assert_ok(&build);
        Fixture {
            _root: root,
            src,
            lib,
            ext,
            run,
        }
    }

    fn run(&self, first: &Path, script: &str) -> Output {
        let path = std::env::join_paths([first, self.lib.as_path()]).expect("a PYTHONPATH");
        host_python()
            .arg("-c")
            .arg(script)
            .env("PYTHONPATH", path)
            .current_dir(&self.run)
            .output()
            .expect("python3 should spawn")
    }

    /// The driver's output against the extension, after asserting it
    /// matches CPython importing the same source.
    fn assert_matches_cpython(&self, script: &str) -> String {
        let compiled = self.run(&self.ext, script);
        let oracle = self.run(&self.src, script);
        assert_ok(&compiled);
        assert_ok(&oracle);
        assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
        stdout_of(&compiled)
    }
}

/// A function redefined before the cycle, a function and a class defined
/// after it.
const MODULE: &str = r#"def early() -> int:
    return 1


def early() -> int:
    return 2


import cb


def late() -> int:
    return 3


class C:
    def __init__(self) -> None:
        self.v = 5

    def get(self) -> int:
        return self.v
"#;

/// Imports `my` back while `my`'s body is at `import cb`.
const HELPER: &str = r#"import my

print("early", my.early())
for name in ("late", "C"):
    try:
        getattr(my, name)
    except AttributeError as e:
        print("AttributeError", str(e).replace(my.__file__, "<file>"))
print("has", hasattr(my, "late"), hasattr(my, "C"))
try:
    from my import late
except ImportError as e:
    print("ImportError", str(e).replace(my.__file__, "<file>"))
"#;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_reentrant_import_sees_only_the_definitions_already_executed() {
    let fixture = Fixture::new("1199_reentrant", MODULE, HELPER);
    let out = fixture.assert_matches_cpython("import my\n");
    // The CPython oracle above is the contract; these pin that it says
    // what #1199 is about rather than agreeing on something vacuous.
    assert!(out.starts_with("early 2\n"), "{out}");
    assert!(out.contains("has no attribute 'late'"), "{out}");
    assert!(out.contains("has no attribute 'C'"), "{out}");
    assert!(out.contains("has False False\n"), "{out}");
    assert!(
        out.contains("ImportError cannot import name 'late'"),
        "{out}"
    );
    assert!(out.contains("partially initialized module 'my'"), "{out}");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_completed_import_exposes_every_export_again_after_a_reimport() {
    let fixture = Fixture::new("1199_complete", MODULE, HELPER);
    let out = fixture.assert_matches_cpython(
        r#"import sys
import my

print(my.early(), my.late(), my.C().get())
print(my.early.__module__, my.late.__module__, my.C.__module__)
del sys.modules["my"]
import my as again

print(again is my, again.early(), again.late(), again.C().get())
print(hasattr(again, "late"), hasattr(again, "C"))
"#,
    );
    assert!(out.contains("2 3 5\nmy my my\n"), "{out}");
    assert!(out.ends_with("False 2 3 5\nTrue True\n"), "{out}");
}

/// `class E(Base): pass` owns no compiled item; it is published once its
/// base's methods are bound, so a cycle after its statement constructs it.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_item_less_subclass_defined_before_the_cycle_is_usable_inside_it() {
    let fixture = Fixture::new(
        "1199_subclass",
        r#"class Base:
    def __init__(self) -> None:
        self.v = 4

    def get(self) -> int:
        return self.v


class E(Base):
    pass


import cb
"#,
        "import my\n\nprint(\"E\", my.E().get(), my.Base().get())\n",
    );
    let out = fixture.assert_matches_cpython("import my\nprint(my.E().get())\n");
    assert_eq!(out, "E 4 4\n4\n");
}

/// Not a CPython comparison: a name redefined after the cycle is hidden
/// until its last definition runs, where CPython would show the first. All
/// definitions share the last one's wrapper, which here accepts a
/// read-only buffer the first definition's body would write into, so
/// publishing the first definition early would hand it that buffer.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_name_redefined_after_the_cycle_is_hidden_until_its_last_definition() {
    let fixture = Fixture::new(
        "1199_redefined",
        r#"def f(b: memoryview) -> float:
    b[0] = 7.0
    return 1.0


import cb


def f(b: memoryview) -> float:
    return b[0]
"#,
        r#"import my

print("has", hasattr(my, "f"))
try:
    my.f(memoryview(bytes(8)).cast("d"))
except AttributeError as e:
    print("AttributeError", str(e).replace(my.__file__, "<file>"))
"#,
    );
    let compiled = fixture.run(
        &fixture.ext,
        "import array\nimport my\n\
         frozen = bytes(8)\n\
         print(my.f(memoryview(frozen).cast('d')), my.f(array.array('d', [6.0])))\n\
         assert frozen == bytes(8)\n",
    );
    assert_ok(&compiled);
    assert_eq!(
        stdout_of(&compiled),
        "has False\nAttributeError partially initialized module 'my' from '<file>' \
         has no attribute 'f' (most likely due to a circular import)\n0.0 6.0\n"
    );
}

/// Drops `my` from `sys.modules` while `my`'s body is at `import cb`, and
/// imports it again: a second `Py_mod_exec` of the same artifact.
const NESTED_EXEC_HELPER: &str = r#"import sys

outer = sys.modules["my"]
del sys.modules["my"]
import my as inner
"#;

/// A nested `Py_mod_exec` of the same artifact, not a CPython comparison
/// for one value. Every compiled static -- each class's type object, the
/// carrier cache, every `fnptr_` slot -- is one per process, so after the
/// inner exec compiled code constructs and recognises that exec's `C`.
/// Each module binds its own exec's class, as before #1199, so
/// `outer.make()` is not an instance of `outer.C`; CPython builds outer's
/// own `C` there.
fn assert_each_module_binds_its_own_exec_class(label: &str, module: &str) {
    let fixture = Fixture::new(label, module, NESTED_EXEC_HELPER);
    let driver = "import my\nimport cb\n\
                  outer, inner = cb.outer, cb.inner\n\
                  print(outer is inner, my is inner, outer.C is inner.C)\n\
                  print(isinstance(outer.make(), outer.C), isinstance(outer.make(), inner.C), \
                  isinstance(inner.make(), inner.C), outer.C().get(), outer.make().get())\n";
    let compiled = fixture.run(&fixture.ext, driver);
    let oracle = fixture.run(&fixture.src, driver);
    assert_ok(&compiled);
    assert_ok(&oracle);
    assert_eq!(
        stdout_of(&compiled),
        "False True False\nFalse True True 5 5\n"
    );
    assert_eq!(
        stdout_of(&oracle),
        "False True False\nTrue False True 5 5\n"
    );
}

/// The cycle runs before the outer class statement, so the outer module
/// binds `C` after the inner exec replaced the type static: it binds the
/// type its own exec snapshotted, not the replacement.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_nested_exec_before_the_class_statement_leaves_each_module_its_own_class() {
    assert_each_module_binds_its_own_exec_class(
        "1199_nested_exec",
        r#"def make() -> object:
    return C()


import cb


class C:
    def __init__(self) -> None:
        self.v = 5

    def get(self) -> int:
        return self.v
"#,
    );
}

/// The cycle runs after the outer class statement, so the outer module
/// already bound its class.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_nested_exec_after_the_class_statement_leaves_each_module_its_own_class() {
    assert_each_module_binds_its_own_exec_class(
        "1199_nested_exec_late",
        r#"class C:
    def __init__(self) -> None:
        self.v = 5

    def get(self) -> int:
        return self.v


def make() -> object:
    return C()


import cb
"#,
    );
}

/// The guard, not a CPython comparison: `make` builds a `D` before the
/// class statement has run (#1490 tracks refusing that), and the instance
/// escapes to the host. Calling `D.m` on it raises a catchable `NameError`
/// instead of calling through the null slot.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_instance_escaping_before_its_class_statement_raises_name_error() {
    let fixture = Fixture::new(
        "1199_guard",
        r#"class Real:
    def __init__(self, v: int) -> None:
        self.v = v


def make() -> object:
    return D(1)


import cb


class D(Real):
    def m(self) -> int:
        return self.v
"#,
        r#"import my

d = my.make()
print("made")
try:
    d.m()
except NameError as e:
    print("NameError", e)
"#,
    );
    let compiled = fixture.run(&fixture.ext, "import my\nprint(my.make().m())\n");
    assert_ok(&compiled);
    assert_eq!(
        stdout_of(&compiled),
        "made\nNameError name 'D.m' is not defined\n1\n"
    );
}

/// An embedded executable runs its program as `__main__` with empty
/// publication tables, so every publication call codegen emits is a no-op
/// and the shared shim's `m_methods = NULL` and post-body safety net bind
/// nothing. The oracle is the interpreter the bundle's `PYCC-BUNDLE`
/// marker records, pinned to 3.14.7 as `tests/issue_1223_embedded_executable.rs`
/// pins it.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn an_embedded_program_with_definitions_runs_like_cpython() {
    let dir = ScratchDir::new("1199_embedded").expect("scratch");
    let source = write(
        &dir,
        "m.py",
        r#"import json


def f() -> int:
    return 1


def f() -> int:
    return 2


class C:
    def __init__(self) -> None:
        self.v = 3

    def get(self) -> int:
        return self.v


class E(C):
    pass


print(json.dumps(f()), C().get(), E().get())
"#,
    );
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert_ok(&build);
    let marker = std::fs::read_to_string(dir.join("app.pycc").join("PYCC-BUNDLE"))
        .expect("an embedded build writes its marker");
    let mut lines = marker.lines();
    assert_eq!(lines.next(), Some("pycc-bundle 1"));
    assert_eq!(lines.next(), Some("python 3.14.7"), "{marker}");
    let python = lines
        .next()
        .and_then(|line| line.strip_prefix("executable "))
        .expect("the marker names its interpreter");
    let run = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    let oracle = Command::new(python)
        .arg(&source)
        .output()
        .expect("CPython runs the oracle program");
    assert_ok(&run);
    assert_ok(&oracle);
    assert_eq!(stdout_of(&run), stdout_of(&oracle));
    assert_eq!(stdout_of(&run), "2 3 3\n");
}
