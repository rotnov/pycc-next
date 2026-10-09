//! Part 1 of #886 (#1394): a `Generic[T, ...]` base whose arguments are type
//! variables -- a module-level `T = TypeVar("T")` or a foreign-imported name
//! -- is admitted and erased (`docs/TYPE_SYSTEM.md`, "Generics"). Every
//! other subscripted or attribute base keeps its explicit `C0001`.
//!
//! The non-ignored tests pin the CLI's accept/refuse split and run one
//! native program. The hosted `--ext` tests are `#[ignore]`d and contribute
//! no line coverage; the Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`, each against CPython's own
//! run of the same source. The lines this change needs covered are covered
//! by `crates/pycc_hir/src/class/generic_base_tests.rs` and the
//! non-ignored tests here.

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

/// Writes `body` to `dir/<file>`, creating its parent directories.
fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

fn check(tag: &str, body: &str) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    pycc()
        .arg("check")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

/// `body` fails `pycc check` with exactly one `C0001` containing `needle`.
fn assert_one_c0001(tag: &str, body: &str, needle: &str) {
    let output = check(tag, body);
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{tag}: {rendered}");
    assert_eq!(rendered.matches("error[").count(), 1, "{tag}: {rendered}");
    assert!(rendered.contains("error[C0001]"), "{tag}: {rendered}");
    assert!(rendered.contains(needle), "{tag}: {rendered}");
}

const PREAMBLE: &str = "from typing import Generic, TypeVar\nT = TypeVar(\"T\")\n";

/// The native program: the erased base leaves `Base` as the only real base,
/// and a subclass of the generic class inherits normally. A value of a type
/// variable is the opaque `object`, so no native value is passed to one
/// (`docs/TYPE_SYSTEM.md`, "Generics").
const PROGRAM: &str = "from typing import Generic, TypeVar\n\
    K = TypeVar(\"K\")\nV = TypeVar(\"V\")\n\
    class Base:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\
    class Pair(Base, Generic[K, V]):\n\
    \x20   def _same(self, key: K) -> K:\n        return key\n\
    class Sub(Pair):\n    def twice(self) -> int:\n        return self.n * 2\n\
    p = Sub(3)\nprint(p.twice())\nprint(isinstance(p, Base))\n";

/// The `--ext` program: `typing.Generic`/`typing.TypeVar` through `import
/// typing`, and foreign `Fraction` values flowing through `K`/`V`.
const EXT_PROGRAM: &str = "import typing\nfrom fractions import Fraction\n\
    K = typing.TypeVar(\"K\")\nV = typing.TypeVar(\"V\")\n\
    class Base:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\
    class Pair(Base, typing.Generic[K, V]):\n\
    \x20   def __init__(self, n: int, key: K, value: V) -> None:\n\
    \x20       super().__init__(n)\n        self.key = key\n        self.value = value\n\
    \x20   def _first(self) -> K:\n        return self.key\n\
    class Sub(Pair):\n    def twice(self) -> int:\n        return self.n * 2\n\
    p = Sub(3, Fraction(1, 2), Fraction(5))\nprint(p.twice())\nprint(p._first())\n\
    print(p.value)\n";

#[test]
fn a_generic_base_over_type_variables_builds_and_runs() {
    let dir = ScratchDir::new("1394_build").expect("scratch");
    let src = write(&dir, "pair.py", PROGRAM);
    let out = dir.join("pair");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
    let run = Command::new(&out)
        .output()
        .expect("the binary should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "6\nTrue\n");
}

#[test]
fn every_refused_generic_shape_keeps_its_c0001() {
    for (tag, body, needle) in [
        (
            "1394_plain",
            "from typing import Generic\nclass C(Generic):\n    pass\n".to_string(),
            "lists a plain `Generic` base",
        ),
        (
            "1394_concrete",
            format!("{PREAMBLE}class C(Generic[int]):\n    pass\n"),
            "`int` in a `Generic[...]` base is not a type variable",
        ),
        (
            "1394_nested",
            format!("{PREAMBLE}class C(Generic[list[T]]):\n    pass\n"),
            "must be the bare name of a type variable",
        ),
        (
            "1394_user_subscript",
            format!("{PREAMBLE}class B:\n    pass\nclass C(B[T]):\n    pass\n"),
            "a base class must be a bare name",
        ),
        (
            "1394_bounded",
            "from typing import TypeVar\nT = TypeVar(\"T\", bound=int)\n".to_string(),
            "a constrained or bounded `TypeVar`",
        ),
        (
            "1394_pep695",
            format!("{PREAMBLE}class C[U](Generic[T]):\n    pass\n"),
            "a PEP 695 generic class is already generic",
        ),
    ] {
        assert_one_c0001(tag, &body, needle);
    }
}

/// The type variable is compile-time only: pycc binds no module global for
/// it, so reading it as a value is an undefined name.
#[test]
fn a_type_variable_is_not_a_runtime_value() {
    let output = check("1394_value", &format!("{PREAMBLE}print(T)\n"));
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(rendered.contains("error[T0021]"), "{rendered}");
}

/// The erased base leaves no `__orig_bases__` or `__parameters__` on the
/// compiled class: reading either, through the class or an instance, is
/// `T0044` (`docs/TYPE_SYSTEM.md`, "Generics").
#[test]
fn the_erased_base_leaves_no_generic_metadata() {
    for (tag, read) in [
        ("1394_orig_bases", "print(C.__orig_bases__)\n"),
        ("1394_parameters", "c = C()\nprint(c.__parameters__)\n"),
        ("1394_inst_orig_bases", "c = C()\nprint(c.__orig_bases__)\n"),
        ("1394_class_parameters", "print(C.__parameters__)\n"),
    ] {
        let body = format!("{PREAMBLE}class C(Generic[T]):\n    pass\n{read}");
        let output = check(tag, &body);
        let rendered = stdout_of(&output);
        assert_eq!(output.status.code(), Some(1), "{tag}: {rendered}");
        assert!(rendered.contains("error[T0044]"), "{tag}: {rendered}");
        assert!(
            rendered.contains("has no attribute named"),
            "{tag}: {rendered}"
        );
    }
}

fn artifact_name() -> &'static str {
    if cfg!(windows) { "m.pyd" } else { "m.abi3.so" }
}

fn import_m(dir: &Path) -> Output {
    host_python()
        .args(["-B", "-c", "import m"])
        .current_dir(dir)
        .env("PYTHONUNBUFFERED", "1")
        .output()
        .expect("python3 should spawn")
}

/// The extension module runs the class like CPython runs the source.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_generic_class_in_an_extension_behaves_like_cpython() {
    let compiled_dir = ScratchDir::new("1394_hosted").expect("scratch");
    let source_dir = ScratchDir::new("1394_hosted_src").expect("scratch");
    let source = write(&source_dir, "m.py", EXT_PROGRAM);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(compiled_dir.join("m"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
    assert!(compiled_dir.join(artifact_name()).is_file());
    let compiled = import_m(&compiled_dir);
    let oracle = import_m(&source_dir);
    for (what, run) in [("pycc", &compiled), ("cpython", &oracle)] {
        assert!(
            run.status.success(),
            "{what}: {}{}",
            stdout_of(run),
            stderr_of(run)
        );
        assert_eq!(stdout_of(run), "6\n1/2\n5\n", "{what}");
    }
}

/// The subject module's shape: `Generic[StateT]` over a `TypeVar` a sibling
/// module binds, through `--foreign-relative-imports`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_generic_base_over_a_relative_import_s_type_variable() {
    let dir = ScratchDir::new("1394_hosted_relative").expect("scratch");
    for (file, text) in [
        ("top/__init__.py", ""),
        ("top/pkg/__init__.py", ""),
        (
            "top/pkg/sib.py",
            "from typing import TypeVar\nStateT = TypeVar('StateT')\n\
             def start():\n    return 's0'\n",
        ),
    ] {
        write(&dir, file, text);
    }
    let body = "from typing import Generic\nfrom .sib import StateT, start\n\
        class _Conf(Generic[StateT]):\n\
        \x20   def __init__(self, start: StateT) -> None:\n        self.start = start\n\
        \x20   def _get(self) -> StateT:\n        return self.start\n\
        print(_Conf(start())._get())\n";
    let source = write(&dir, "src/m.py", body);
    std::fs::create_dir_all(dir.join("stage")).expect("mkdir stage");
    // Built outside the package tree so the artifact cannot shadow the
    // `.py` oracle (CPython tries extension loaders first).
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("stage").join("m"))
        .args(["--ext", "--foreign-relative-imports"])
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
    let run = || {
        host_python()
            .args(["-B", "-c", "import top.pkg.m"])
            .current_dir(&*dir)
            .env("PYTHONUNBUFFERED", "1")
            .output()
            .expect("python3 should spawn")
    };
    let installed_source = dir.join("top/pkg/m.py");
    std::fs::copy(&source, &installed_source).expect("install m.py");
    let oracle = run();
    std::fs::remove_file(&installed_source).expect("remove m.py");
    std::fs::copy(
        dir.join("stage").join(artifact_name()),
        dir.join("top/pkg").join(artifact_name()),
    )
    .expect("install the artifact");
    let compiled = run();
    for (what, output) in [("cpython", &oracle), ("pycc", &compiled)] {
        assert!(
            output.status.success(),
            "{what}: {}{}",
            stdout_of(output),
            stderr_of(output)
        );
        assert_eq!(stdout_of(output), "s0\n", "{what}");
    }
}
