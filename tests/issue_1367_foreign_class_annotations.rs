//! Part 1 of #1367: a class a foreign import binds is spellable in an
//! annotation. `token: Token` on a parameter, a return, a local, a module
//! global or an instance attribute resolves to the opaque `object`
//! (`docs/TYPE_SYSTEM.md`, the `object` row); a subscript on it is erased
//! without resolving its arguments, and nothing is checked at run time,
//! which is what CPython does with an annotation.
//!
//! The non-ignored tests pin, through the CLI, the positions that stay
//! refused and the codes they are refused with. The hosted tests are
//! `#[ignore]`d and contribute no line coverage; the Tier-1
//! `native-build-test` leg runs them with `cargo test --workspace --
//! --include-ignored`. Each compares the artifact against CPython's own run
//! of the same source, so no CPython message text is transcribed. The lines
//! this change needs covered are covered by the non-ignored tests here, by
//! `crates/pycc_hir/src/import/tests/annotations_foreign.rs`,
//! `crates/pycc_hir/src/class/declared_attrs_tests.rs`,
//! `crates/pycc_hir/src/class/init_slot.rs`'s own tests and
//! `crates/pycc_codegen/src/attr_slot.rs`'s own tests.

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

/// `pycc check` on `body` written as `m.py` in a fresh scratch directory.
fn check(tag: &str, body: &str) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    pycc()
        .arg("check")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

fn assert_checks(tag: &str, body: &str) {
    let output = check(tag, body);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{tag}: {}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// `body` fails `pycc check` with exactly one error, `code`, whose message
/// contains `needle`.
fn assert_one_error(tag: &str, body: &str, code: &str, needle: &str) {
    let output = check(tag, body);
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{tag}: {rendered}");
    assert_eq!(rendered.matches("error[").count(), 1, "{tag}: {rendered}");
    assert!(
        rendered.contains(&format!("error[{code}]")),
        "{tag}: {rendered}"
    );
    assert!(rendered.contains(needle), "{tag}: {rendered}");
}

const FRACTION: &str = "from fractions import Fraction\n";

#[test]
fn check_accepts_every_admitted_position() {
    assert_checks(
        "1367_check_positions",
        "from fractions import Fraction\nfrom queue import Queue\n\
         x: Fraction = Fraction(1, 2)\n\
         def _pick(f: Fraction) -> Fraction:\n    y: Fraction = f\n    return y\n\
         class _Box:\n    f: Fraction\n\
         \x20   def __init__(self, f: Fraction, q: Queue[int]) -> None:\n\
         \x20       self.f = f\n        self.q = q\n\
         b = _Box(_pick(x), Queue())\n",
    );
}

/// The subscript's arguments are never resolved: `Nope` names nothing, and
/// CPython 3.14 never evaluates the annotation either (PEP 649).
#[test]
fn a_subscript_on_a_foreign_class_is_erased_unresolved() {
    assert_checks(
        "1367_check_erased",
        "from queue import Queue\ndef _f(t: Queue[Nope]) -> int:\n    return 1\n",
    );
}

/// A D-135 alias of a foreign class is an alias of `object`, and a second
/// module importing the alias sees `object` too.
#[test]
fn a_type_alias_of_a_foreign_class_crosses_modules() {
    let dir = ScratchDir::new("1367_alias_dep").expect("scratch");
    write(
        &dir,
        "dep.py",
        "from fractions import Fraction\ntype F = Fraction\n",
    );
    let output = pycc()
        .arg("check")
        .arg(write(
            &dir,
            "m.py",
            "from dep import F\nfrom fractions import Fraction\n\
             def _f(t: F) -> F:\n    return t\nn = _f(Fraction(1, 2))\n",
        ))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(0), "{}", stdout_of(&output));
}

/// A container, a union or a tuple of a foreign class is not compiled: each
/// is refused with the code that names the unsupported element type.
#[test]
fn a_foreign_class_nested_in_a_container_keeps_its_element_code() {
    for (tag, annotation, code, needle) in [
        (
            "1367_dict",
            "dict[str, Fraction]",
            "T0036",
            "dict[str, object] is not compiled yet",
        ),
        (
            "1367_set",
            "set[Fraction]",
            "T0038",
            "set[object] is not compiled yet",
        ),
        (
            "1367_list",
            "list[Fraction]",
            "T0034",
            "list[object] is not compiled yet",
        ),
        (
            "1367_optional",
            "Fraction | None",
            "T0049",
            "`Optional[object]` is not supported yet",
        ),
        (
            "1367_tuple",
            "tuple[Fraction, int]",
            "T0039",
            "tuple element type `object` is not compiled yet",
        ),
    ] {
        assert_one_error(
            tag,
            &format!("{FRACTION}def _f(t: {annotation}) -> int:\n    return 1\n"),
            code,
            needle,
        );
    }
}

/// The positions Part 1 does not widen: `object` itself (the top type is
/// Part 3, #1387), an annotated attribute target (#891), and a container
/// argument, which the opaque type does not accept (a scalar, `None` or
/// class-instance argument is boxed since #1475,
/// `tests/issue_1475_object_boxing.rs`).
#[test]
fn the_positions_outside_part_1_are_refused() {
    assert_one_error(
        "1367_object",
        "def _f(t: object) -> int:\n    return 1\n",
        "C0001",
        "type annotation `object` is not supported yet",
    );
    assert_one_error(
        "1367_annotated_attr",
        &format!(
            "{FRACTION}class _C:\n    def __init__(self, f: Fraction) -> None:\n\
             \x20       self.f: Fraction = f\n"
        ),
        "C0001",
        "an annotated attribute target annotated `object` with this value is not supported yet",
    );
    assert_one_error(
        "1367_list_argument",
        &format!("{FRACTION}def _f(t: Fraction) -> int:\n    return 1\nn = _f([3])\n"),
        "T0021",
        "argument 1 of `_f` expects `object`, got `list[int]`",
    );
}

/// A PEP 695 generic taking a foreign object keeps I0404, with and without
/// a `T` return. The plan predicted `T0022` for the `-> T` form; both give
/// I0404 today. Separately, an unannotated non-`T` parameter of a generic
/// (#1390) and a foreign class call inside any generic (#1391) are their own
/// issues.
#[test]
fn a_generic_taking_a_foreign_object_keeps_i0404() {
    for (tag, signature, body) in [
        ("1367_generic_int", "-> int", "return 1"),
        ("1367_generic_t", "-> T", "return t"),
    ] {
        assert_one_error(
            tag,
            &format!(
                "{FRACTION}def _g[T](t: T, f: Fraction) {signature}:\n    {body}\n\
                 n = _g(1, Fraction(1, 2))\n"
            ),
            "I0404",
            "passing a CPython object to a generic function is not supported yet",
        );
    }
}

/// `check` accepts a public function taking a foreign object, and since
/// #1397 (D-258 rule 5) `build --ext` no longer refuses to export it: the
/// boundary carries the object itself, unchecked against the class, which
/// Part 2 (#1386, D-244's #1386 amendment) decided.
/// The build now reaches the C step; only the absence of `C0003` and of a
/// type error is pinned, so the test does not depend on that step
/// succeeding. `tests/issue_1397_ext_any_object.rs` runs the artifact.
#[test]
fn a_public_function_taking_a_foreign_object_is_no_longer_refused_at_the_boundary() {
    let dir = ScratchDir::new("1367_public").expect("scratch");
    let source = write(
        &dir,
        "m.py",
        &format!("{FRACTION}def f(t: Fraction) -> int:\n    return 1\n"),
    );
    let check = pycc()
        .arg("check")
        .arg(&source)
        .output()
        .expect("pycc should spawn");
    assert_eq!(check.status.code(), Some(0), "{}", stdout_of(&check));
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("m"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    let rendered = format!("{}{}", stdout_of(&build), stderr_of(&build));
    assert!(!rendered.contains("C0003"), "{rendered}");
    assert!(!rendered.contains("error[T"), "{rendered}");
}

// ---------------------------------------------------------------------
// Hosted: the artifact against CPython's own run of the same source.
// ---------------------------------------------------------------------

/// The artifact's file name for `-o <dir>/m`: `.pyd` on Windows and
/// `.abi3.so` elsewhere (`docs/CLI_SPEC.md`).
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

/// Builds `body` as the extension `m` and imports it, then imports the same
/// source as `m.py` in CPython, and asserts both succeed with the same
/// stdout, which is returned.
fn assert_matches_cpython(tag: &str, body: &str) -> String {
    let compiled_dir = ScratchDir::new(tag).expect("scratch");
    let source_dir = ScratchDir::new(&format!("{tag}_src")).expect("scratch");
    let source = write(&source_dir, "m.py", body);
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
    }
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    stdout_of(&compiled)
}

/// A foreign class on a parameter, a return, a local, a declared and an
/// undeclared instance attribute read from a subclass method, and a
/// truth-tested `Queue[int]` parameter.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_foreign_class_annotation_behaves_like_cpython() {
    let stdout = assert_matches_cpython(
        "1367_hosted_fraction",
        "from fractions import Fraction\nfrom queue import Queue\n\
         def _pick(f: Fraction) -> Fraction:\n    x: Fraction = f\n    return x\n\
         class _Box:\n    f: Fraction\n\
         \x20   def __init__(self, f: Fraction) -> None:\n\
         \x20       self.f = f\n        self.g = f\n\
         \x20   def small(self) -> Fraction:\n        return self.f.limit_denominator(10)\n\
         \x20   def reset(self, f: Fraction) -> None:\n        self.f = f\n\
         class _Sub(_Box):\n    def show(self) -> None:\n        print(self.f)\n\
         def _q(q: Queue[int]) -> int:\n    if q:\n        return 1\n    return 0\n\
         b = _Sub(_pick(Fraction(3, 4)))\n\
         print(str(b.f) + '|' + str(b.g) + '|' + str(b.small()))\n\
         b.reset(Fraction(5, 1))\nb.show()\nn = _q(Queue())\nprint(n)\n",
    );
    assert_eq!(stdout, "3/4|3/4|3/4\n5\n1\n");
}

/// `Queue[int]` is erased to the class itself, so the queue's own methods
/// run on the parameter.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_subscripted_foreign_class_parameter_is_the_object_itself() {
    let stdout = assert_matches_cpython(
        "1367_hosted_queue",
        "from queue import Queue\n\
         def _put(q: Queue[int], n: int) -> None:\n    q.put(n)\n\
         q = Queue()\n_put(q, 3)\n_put(q, 4)\nprint(q.qsize())\n",
    );
    assert_eq!(stdout, "2\n");
}

/// #1138's dotted channel binds an annotatable class too.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_dotted_import_s_class_annotates_a_parameter() {
    let stdout = assert_matches_cpython(
        "1367_hosted_dotted",
        "from json.decoder import JSONDecoder\n\
         def _d(d: JSONDecoder) -> None:\n    print(d.decode('[1]'))\n\
         _d(JSONDecoder())\n",
    );
    assert_eq!(stdout, "[1]\n");
}

/// #1366's relative channel: a sibling module's class and its `TypeVar`
/// both annotate, and a `TypeVar`-annotated attribute holds any object.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_relative_import_s_class_and_typevar_annotate() {
    let dir = ScratchDir::new("1367_hosted_relative").expect("scratch");
    for (file, text) in [
        ("top/__init__.py", ""),
        ("top/pkg/__init__.py", ""),
        (
            "top/pkg/sib.py",
            "from typing import TypeVar\nStateT = TypeVar('StateT')\n\
             class Tok:\n    def __init__(self, name):\n        self.name = name\n",
        ),
    ] {
        write(&dir, file, text);
    }
    let body = "from .sib import Tok, StateT\n\
        class _Holder:\n    tok: Tok\n\
        \x20   def __init__(self, tok: Tok, s: StateT) -> None:\n\
        \x20       self.tok = tok\n        self.s = s\n\
        \x20   def show(self) -> None:\n        print(self.tok.name)\n        print(self.s.name)\n\
        h = _Holder(Tok('a'), Tok('b'))\nh.show()\n";
    let source = write(&dir, "src/m.py", body);
    std::fs::create_dir_all(dir.join("stage")).expect("mkdir stage");
    // Built outside the package tree: CPython's finder tries extension
    // loaders before source loaders, so an artifact already in `top/pkg`
    // would shadow the `.py` oracle.
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("stage").join("m"))
        .args(["--ext", "--foreign-relative-imports"])
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
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
        assert_eq!(stdout_of(output), "a\nb\n", "{what}");
    }
}

/// A private helper in a sibling project module takes and returns a
/// foreign-class-annotated value across the module boundary.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_cross_module_helper_takes_a_foreign_class_parameter() {
    let compiled_dir = ScratchDir::new("1367_hosted_cross_module").expect("scratch");
    let source_dir = ScratchDir::new("1367_hosted_cross_module_src").expect("scratch");
    let dep = "from fractions import Fraction\n\
        def _num(f: Fraction) -> Fraction:\n    print(f.numerator)\n    return f\n";
    for dir in [&compiled_dir, &source_dir] {
        write(dir, "dep.py", dep);
    }
    let source = write(
        &source_dir,
        "m.py",
        "from fractions import Fraction\nfrom dep import _num\nprint(_num(Fraction(3, 4)))\n",
    );
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(compiled_dir.join("m"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    for (what, dir) in [("pycc", &compiled_dir), ("cpython", &source_dir)] {
        let run = import_m(dir);
        assert!(run.status.success(), "{what}: {}", stderr_of(&run));
        assert_eq!(stdout_of(&run), "3\n3/4\n", "{what}");
    }
}

/// A foreign-class slot read before `__init__` assigns it raises CPython's
/// own `AttributeError` (#1388; before it, the NULL word made the foreign
/// operation report `SystemError`, the uninitialised-slot hazard #1148).
/// The artifact's last traceback line must equal CPython's.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_foreign_slot_read_before_its_assignment_raises_attribute_error() {
    let compiled_dir = ScratchDir::new("1367_hosted_null_slot").expect("scratch");
    let source_dir = ScratchDir::new("1367_hosted_null_slot_src").expect("scratch");
    let source = write(
        &source_dir,
        "m.py",
        "from fractions import Fraction\n\
         class _Box:\n\
         \x20   def __init__(self, f: Fraction) -> None:\n\
         \x20       self.show()\n        self.f = f\n\
         \x20   def show(self) -> None:\n        print(self.f.numerator)\n\
         _Box(Fraction(1, 2))\n",
    );
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(compiled_dir.join("m"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let last_lines: Vec<String> = [&compiled_dir, &source_dir]
        .into_iter()
        .map(|dir| {
            let run = import_m(dir);
            assert!(!run.status.success(), "{}", stdout_of(&run));
            let stderr = stderr_of(&run);
            stderr.lines().last().unwrap_or_default().to_string()
        })
        .collect();
    assert_eq!(
        last_lines[0], "AttributeError: '_Box' object has no attribute 'f'",
        "pycc"
    );
    assert_eq!(last_lines[0], last_lines[1], "pycc vs cpython");
}

/// A PEP 695 type parameter resolves before the alias table, so `[Fraction]`
/// shadows the foreign `Fraction` inside that generic, as in CPython: the
/// call instantiates it at `int`.
#[test]
fn a_pep_695_type_parameter_shadows_a_foreign_name() {
    assert_checks(
        "1367_type_param_shadow",
        "from fractions import Fraction\n\
         def _g[Fraction](t: Fraction) -> Fraction:\n    return t\n\
         n = _g(1)\nprint(n + 1)\n",
    );
}
