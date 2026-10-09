//! End-to-end proof for receiver-exact inherited methods ([#1337], D-254).
//!
//! An inherited method runs the body its receiver's static class resolves
//! to: `self.m()`, `super()`, class-attribute reads, property accessors,
//! `isinstance(self, ...)` and classmethod calls inside an inherited body
//! are decided for the subclass, as CPython decides them at run time. The
//! byte-exact oracle fixtures are `tests/fixtures/receiver_exact_dispatch.py`
//! and `tests/fixtures/receiver_exact_protocol.py` (registered in
//! `tests/conformance/classes.rs`); this file pins their expected output
//! without the oracle, the honest refusals where a copy cannot be typed, and
//! the hosted `--ext` arm.
//!
//! [#1337]: https://github.com/rotnov/pycc/issues/1337

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// Builds `source` and returns the built program's stdout.
fn build_and_run(category: &str, source: &str) -> String {
    let dir = ScratchDir::new(category).expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write the subject");
    let exe = dir.join("subject");
    let build = pycc()
        .arg("build")
        .arg(&path)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{source}: {}", text(&build.stderr));
    let run: Output = Command::new(&exe)
        .output()
        .expect("the program should spawn");
    assert!(run.status.success(), "{source}: {}", text(&run.stderr));
    text(&run.stdout)
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .expect("read the fixture")
}

/// The oracle fixture's CPython 3.14.7 output, pinned so the proof runs
/// without the oracle too.
#[test]
fn the_dispatch_fixture_prints_what_cpython_prints() {
    assert_eq!(
        build_and_run("e2e_1337_dispatch", &fixture("receiver_exact_dispatch.py")),
        "2 1\n2 12\n20\n2 3 1\nTrue False\n6 20 10\n3 2\n13 12 1\n7\n42\n111 1111 11\n\
         r! 1!\n2\n3 2\n1\n2\n2 12\n101 1\n"
    );
}

#[test]
fn the_protocol_fixture_prints_what_cpython_prints() {
    assert_eq!(
        build_and_run("e2e_1337_protocol", &fixture("receiver_exact_protocol.py")),
        "2 1\n102 101\n"
    );
}

/// The issue's own reproduction, verbatim: an inherited `call_m` calls
/// `self.m()`, which the subclass overrides. CPython 3.14 prints `2` (the
/// oracle fixture's `c02_chain` case checks the same shape byte for byte).
#[test]
fn the_issue_reproduction_runs_the_override() {
    let source = "class A:\n    def m(self) -> int:\n        return 1\n\n    \
                  def call_m(self) -> int:\n        return self.m()\n\n\n\
                  class B(A):\n    def m(self) -> int:\n        return 2\n\n\n\
                  print(B().call_m())\n";
    assert_eq!(build_and_run("e2e_1337_issue", source), "2\n");
}

/// The base class lives in an imported module and the subclass in the
/// program (plan item 15): the copy pass runs over the linked program, so
/// the inherited `call_m` still runs the subclass's `m`. CPython prints
/// `1 2`.
///
/// A subclass defined in a *second* imported module whose base comes from
/// a third one is not covered here: that import shape panics in
/// `pycc_hir::import` on `main` already, independently of #1337 (#1351).
#[test]
fn an_imported_base_runs_the_subclass_override() {
    let dir = ScratchDir::new("e2e_1337_multi_file").expect("scratch");
    std::fs::write(
        dir.join("m1.py"),
        "class A:\n    def m(self) -> int:\n        return 1\n\n    \
         def call_m(self) -> int:\n        return self.m()\n",
    )
    .expect("write m1");
    let main = dir.join("main.py");
    std::fs::write(
        &main,
        "from m1 import A\n\n\nclass B(A):\n    def m(self) -> int:\n        return 2\n\n\n\
         print(A().call_m(), B().call_m())\n",
    )
    .expect("write main");
    let exe = dir.join("main");
    let build = pycc()
        .arg("build")
        .arg(&main)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let run = Command::new(&exe)
        .output()
        .expect("the program should spawn");
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), "1 2\n");
}

/// A copy's diagnostic is attributed to the file that defines the origin
/// body, not to the file whose subclass needed the copy: the escape is
/// written in `m1.py`, so the rendered span names `m1.py`.
#[test]
fn a_copy_diagnostic_names_the_origin_file() {
    let dir = ScratchDir::new("e2e_1337_multi_file_refusal").expect("scratch");
    std::fs::write(
        dir.join("m1.py"),
        "class A:\n    def m(self) -> int:\n        return 1\n\n    \
         def me(self) -> A:\n        return self\n",
    )
    .expect("write m1");
    let main = dir.join("main.py");
    std::fs::write(
        &main,
        "from m1 import A\n\n\nclass B(A):\n    def m(self) -> int:\n        return 2\n\n\n\
         print(B().me().m())\n",
    )
    .expect("write main");
    let output = pycc()
        .arg("check")
        .arg("--error-format")
        .arg("json")
        .arg(&main)
        .output()
        .expect("pycc should spawn");
    assert!(!output.status.success());
    let rendered = format!("{}{}", text(&output.stdout), text(&output.stderr));
    assert!(rendered.contains("\"T0022\""), "{rendered}");
    assert!(
        rendered.contains("while compiling `A.me` inherited by subclass `B`"),
        "{rendered}"
    );
    assert!(rendered.contains("m1.py"), "{rendered}");
    assert!(!rendered.contains("main.py"), "{rendered}");
}

/// The template-method shape the issue generalizes: a base method calls a
/// hook the subclass overrides, and the base itself still runs its own hook.
#[test]
fn a_template_method_runs_the_subclass_hook() {
    let source = "class Base:\n    def run(self) -> str:\n        return self.step()\n    \
                  def step(self) -> str:\n        return \"base\"\n\n\
                  class Child(Base):\n    def step(self) -> str:\n        return \"child\"\n\n\
                  print(Child().run())\nprint(Base().run())\n";
    assert_eq!(build_and_run("e2e_1337_template", source), "child\nbase\n");
}

/// A `super()` inside an inherited body continues along the *receiver's*
/// MRO: in a diamond, `B.f`'s `super()` reaches the sibling `C` for a `D`.
#[test]
fn super_in_an_inherited_body_follows_the_receivers_mro() {
    let source = "class A:\n    def f(self) -> str:\n        return \"A\"\n\
                  class B(A):\n    def f(self) -> str:\n        return \"B->\" + super().f()\n\
                  class C(A):\n    def f(self) -> str:\n        return \"C\"\n\
                  class D(B, C):\n    def f(self) -> str:\n        return \"D->\" + super().f()\n\
                  print(D().f())\nprint(B().f())\n";
    assert_eq!(build_and_run("e2e_1337_diamond", source), "D->B->C\nB->A\n");
}

/// Runs `pycc check --error-format json` and returns the output, asserting
/// the check failed.
fn check_json_fails(category: &str, source: &str) -> String {
    let dir = ScratchDir::new(category).expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write the subject");
    let output = pycc()
        .arg("check")
        .arg("--error-format")
        .arg("json")
        .arg(&path)
        .output()
        .expect("pycc should spawn");
    assert!(!output.status.success(), "{source} was accepted");
    let mut rendered = text(&output.stdout);
    rendered.push_str(&text(&output.stderr));
    rendered
}

/// Where a copy compiled for the subclass cannot be typed -- the receiver
/// escapes into a slot typed as the base, or the body uses a capability the
/// subclass lacks -- the program is refused, never compiled for the base
/// class, and the diagnostic names the subclass the body was compiled for.
#[test]
fn an_untypeable_copy_is_refused_and_names_the_subclass() {
    for (source, code, note) in [
        (
            "class A:\n    def m(self) -> int:\n        return 1\n    def me(self) -> A:\n        \
             return self\nclass B(A):\n    def m(self) -> int:\n        return 2\n\
             print(B().me().m())\n",
            "\"T0022\"",
            "while compiling `A.me` inherited by subclass `B`",
        ),
        (
            "class A:\n    def m(self) -> int:\n        return 1\n    def g(self) -> int:\n        \
             return helper(self)\nclass B(A):\n    def m(self) -> int:\n        return 2\n\
             def helper(a: A) -> int:\n    return a.m()\nprint(B().g())\n",
            "\"T0021\"",
            "while compiling `A.g` inherited by subclass `B`",
        ),
        (
            "from dataclasses import dataclass\n@dataclass\nclass A:\n    x: int\n    \
             def show(self) -> None:\n        print(self)\nclass B(A):\n    pass\nB(1).show()\n",
            "\"C0001\"",
            "while compiling `A.show` inherited by subclass `B`",
        ),
        (
            "from dataclasses import dataclass\n@dataclass\nclass A:\n    x: int\n    \
             def same(self, o: A) -> bool:\n        return self == o\nclass B(A):\n    pass\n\
             print(B(1).same(A(1)))\n",
            "\"T0021\"",
            "while compiling `A.same` inherited by subclass `B`",
        ),
        (
            "class A:\n    def m(self) -> int:\n        return 1\n    def g(self) -> int:\n        \
             x: A = self\n        return x.m()\nclass B(A):\n    def m(self) -> int:\n        \
             return 2\nprint(B().g())\n",
            "\"T0025\"",
            "while compiling `A.g` inherited by subclass `B`",
        ),
        (
            "class A:\n    def me(self) -> A:\n        return self\nclass B(A):\n    pass\n\
             def chk(a: A) -> bool:\n    return isinstance(a, B)\nprint(chk(B().me()))\n",
            "\"T0022\"",
            "while compiling `A.me` inherited by subclass `B`",
        ),
    ] {
        let rendered = check_json_fails("e2e_1337_refused", source);
        assert!(rendered.contains(code), "{source}: {rendered}");
        assert!(rendered.contains(note), "{source}: {rendered}");
    }
}

/// A class-attribute read through `self` in an inherited body reads the
/// receiver's class attribute (plan item 3).
#[test]
fn an_inherited_body_reads_the_receivers_class_attribute() {
    let source = "class A:\n    X: int = 1\n    def g(self) -> int:\n        return self.X\n\
                  class B(A):\n    X: int = 2\nprint(A().g(), B().g())\n";
    assert_eq!(build_and_run("e2e_1337_class_attr", source), "1 2\n");
}

/// An exception raised inside a copy names the method, not a mangled copy
/// name, in the uncaught-exception frame (plan item 16).
#[test]
fn an_exception_raised_in_a_copy_names_the_method_frame() {
    let dir = ScratchDir::new("e2e_1337_frame").expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(
        &path,
        "class A:\n    def m(self) -> int:\n        return 1\n    def g(self) -> int:\n        \
         if self.m() == 2:\n            raise ValueError(\"boom\")\n        return 0\n\
         class B(A):\n    def m(self) -> int:\n        return 2\nprint(B().g())\n",
    )
    .expect("write the subject");
    let exe = dir.join("subject");
    let build = pycc()
        .arg("build")
        .arg(&path)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let run = Command::new(&exe)
        .output()
        .expect("the program should spawn");
    assert_eq!(run.status.code(), Some(1));
    let stderr = text(&run.stderr);
    assert!(stderr.contains("in g\n"), "{stderr}");
    assert!(stderr.ends_with("ValueError: boom\n"), "{stderr}");
}

/// A diagnostic raised identically in an origin body and its copies is
/// reported once, at the origin.
#[test]
fn a_diagnostic_repeated_in_every_copy_is_reported_once() {
    let source = "class A:\n    def m(self) -> int:\n        return 1\n    def g(self) -> int:\n        \
                  return self.m() + \"x\"\nclass B(A):\n    def m(self) -> int:\n        return 2\n\
                  print(B().g())\n";
    let rendered = check_json_fails("e2e_1337_dedup", source);
    assert_eq!(rendered.matches("\"code\"").count(), 1, "{rendered}");
}

const EXT_MODULE: &str = "class A:\n    def __init__(self) -> None:\n        self.x = 0\n        \
     self.x = self.hook()\n    def hook(self) -> int:\n        return 1\n    def m(self) -> int:\n        \
     return 1\n    def g(self) -> int:\n        return self.m()\n    @classmethod\n    \
     def k(cls) -> int:\n        return cls.base()\n    @classmethod\n    def base(cls) -> int:\n        \
     return 10\n    def getx(self) -> int:\n        return self.x\n    X: int = 1\n    \
     def cx(self) -> int:\n        return self.X\n    @property\n    def p(self) -> int:\n        \
     return self.m()\n\
     class B(A):\n    def m(self) -> int:\n        return 2\n    def hook(self) -> int:\n        \
     return 7\n    @classmethod\n    def base(cls) -> int:\n        return 20\n    X: int = 3\n";

const EXT_SCRIPT: &str = "import pycc_1337_mod as mod\n\
     b = mod.B()\n\
     a = mod.A()\n\
     print(b.g(), a.g(), b.getx(), a.getx(), mod.B.k(), mod.A.k())\n\
     print(b.cx(), a.cx(), b.p, a.p, callable(mod.B.__dict__[\"p\"]))\n";

/// A host calling an inherited method on a published subclass runs the
/// copy compiled for that subclass, and constructing it runs the copied
/// `__init__`, and a copied body reads the subclass's class attribute. A
/// copied `@property` getter is not published as a plain method: since
/// #1442 it answers the attribute through a read-only getset descriptor
/// that runs the subclass's copy (`docs/RUNTIME.md`).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_hosted_subclass_runs_its_receiver_exact_copies() {
    let dir = ScratchDir::new("e2e_1337_ext").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, EXT_MODULE).expect("write the module");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_1337_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(EXT_SCRIPT)
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), "2 1 7 1 20 10\n3 1 2 1 False\n");
}

const STR_MIXIN: &str =
    "class Mixin:\n    def __str__(self) -> str:\n        return \"custom\"\n\n";

/// A raised exception value is a runtime exception object that never calls a
/// user dunder, so an exception class whose MRO resolves one to a user class
/// is refused at its definition rather than printing the message where
/// CPython prints the override (WI-6b).
#[test]
fn a_user_dunder_an_exception_value_would_ignore_is_refused() {
    for (source, dunder) in [
        (
            format!(
                "{STR_MIXIN}class E(Mixin, ValueError):\n    pass\n\ntry:\n    raise E(\"x\")\n\
                 except ValueError as e:\n    print(e)\n"
            ),
            "`__str__` (defined by `Mixin`)",
        ),
        (
            "class E(ValueError):\n    def __bool__(self) -> bool:\n        return False\n\n\
             try:\n    raise E(\"x\")\nexcept ValueError as e:\n    print(\"t\" if e else \"f\")\n"
                .to_string(),
            "`E` has a user-defined `__bool__`",
        ),
    ] {
        let rendered = check_json_fails("e2e_1337_exception_dunder", &source);
        assert!(rendered.contains("\"C0001\""), "{source}: {rendered}");
        assert!(rendered.contains(dunder), "{source}: {rendered}");
        assert!(rendered.contains("Part 3 of #541"), "{source}: {rendered}");
    }
}

/// With the builtin base first, `BaseException.__str__` wins over the mixin,
/// exactly as in CPython, and the program is accepted.
#[test]
fn a_builtin_base_before_a_str_mixin_prints_the_message() {
    let source = format!(
        "{STR_MIXIN}class E(ValueError, Mixin):\n    pass\n\ntry:\n    raise E(\"x\")\n\
         except ValueError as e:\n    print(e)\n    print(f\"{{e}}\")\n"
    );
    assert_eq!(build_and_run("e2e_1337_exception_mixin", &source), "x\nx\n");
}

/// `isinstance` on a caught builtin exception value reads its runtime type
/// tag (WI-6a); the fixture's CPython 3.14.7 output, pinned without the
/// oracle (registered in `tests/conformance/classes.rs`).
#[test]
fn the_exception_isinstance_fixture_prints_what_cpython_prints() {
    assert_eq!(
        build_and_run(
            "e2e_1337_exc_isinstance",
            &fixture("exception_isinstance.py")
        ),
        "True False True\nTrue True False\nFalse True\nTrue True False\nTrue False\n\
         False False False\nTrue True False\nTrue False\nTrue True False\nTrue False\n\
         False True True\n"
    );
}
