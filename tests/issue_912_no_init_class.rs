//! #912 / [D-225]: a class that declares no `__init__` and inherits none
//! through its MRO gets an implicit zero-argument constructor synthesized at
//! HIR-lowering time, so it is instantiable exactly as CPython's inherited
//! `object.__init__` makes it.
//!
//! [D-225]: ../docs/decisions/D-225-synthesize-an-implicit-zero-argument-constructor.md

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

/// Build `source` and run the resulting binary, asserting it prints
/// `expected_stdout`.
fn build_and_run(slug: &str, source: &str, expected_stdout: &str) {
    let dir = ScratchDir::new(slug).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "prog.py", source);
    let out = dir.join("prog");

    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "pycc build should succeed, got: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&out).output().unwrap();
    assert!(run.status.success(), "the built program should exit 0");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        expected_stdout,
        "unexpected program output"
    );
}

/// Build `source`, asserting the build fails and its stderr contains
/// `expected_fragment`. Asserted on the message rather than the span: these
/// diagnostics are emitted with a zero span and render against line 1.
fn build_fails_with(slug: &str, source: &str, expected_fragment: &str) {
    let dir = ScratchDir::new(slug).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "prog.py", source);
    let out = dir.join("prog");

    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!build.status.success(), "pycc build should fail");
    let stderr = String::from_utf8_lossy(&build.stderr);
    assert!(
        stderr.contains(expected_fragment),
        "stderr should contain {expected_fragment:?}, got: {stderr}"
    );
    assert!(!out.exists(), "a failing build must not leave a binary");
}

/// The issue's own acceptance snippet: a class whose body is nothing but the
/// #885/D-224 class attributes it was written to hold.
#[test]
fn a_class_attribute_only_class_is_instantiable() {
    build_and_run(
        "912_config",
        "class Config:\n    MAX: int = 10\n    NAME: str = \"cfg\"\n\n\nc = Config()\nprint(c.MAX)\nprint(Config.NAME)\n",
        "10\ncfg\n",
    );
}

/// The minimal shape: an empty class body.
#[test]
fn a_bare_pass_class_is_instantiable() {
    build_and_run(
        "912_bare",
        "class A:\n    pass\n\n\na = A()\nprint(1)\n",
        "1\n",
    );
}

/// A methods-only class: the synthesized constructor establishes no
/// attribute slots, but instance-method dispatch is unaffected.
#[test]
fn a_methods_only_class_is_instantiable_and_dispatches() {
    build_and_run(
        "912_methods",
        "class Greeter:\n    def greet(self) -> str:\n        return \"hi\"\n\n\ng = Greeter()\nprint(g.greet())\n",
        "hi\n",
    );
}

/// A `@property`-only class -- properties live outside the method table, so
/// this class reaches `ensure_init` with an empty one.
#[test]
fn a_property_only_class_is_instantiable() {
    build_and_run(
        "912_property",
        "class P:\n    @property\n    def v(self) -> int:\n        return 3\n\n\np = P()\nprint(p.v)\n",
        "3\n",
    );
}

/// A `@staticmethod`-only namespace class: its static method is mangled
/// separately, so the method table is likewise empty at `ensure_init`.
#[test]
fn a_staticmethod_only_class_is_instantiable() {
    build_and_run(
        "912_static",
        "class NS:\n    @staticmethod\n    def f() -> int:\n        return 4\n\n\nn = NS()\nprint(NS.f())\n",
        "4\n",
    );
}

/// A base class with no `__init__` whose derived class declares one: the
/// derived class keeps its own constructor, and the synthesized base
/// constructor does not interfere with the derived attribute slots.
#[test]
fn a_derived_class_may_declare_its_own_init_over_a_synthesized_base() {
    build_and_run(
        "912_derived_init",
        "class Base:\n    pass\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.x = 2\n\n\nd = Derived()\nprint(d.x)\n",
        "2\n",
    );
}

/// `super().__init__()` resolves into the base class's *synthesized*
/// constructor exactly as it would into a hand-written one.
#[test]
fn super_init_reaches_a_synthesized_base_constructor() {
    build_and_run(
        "912_super",
        "class Base:\n    pass\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        super().__init__()\n        self.x = 1\n\n\nd = Derived()\nprint(d.x)\n",
        "1\n",
    );
}

/// A chain of `__init__`-less classes: `B` inherits `A`'s synthesized
/// constructor rather than synthesizing a second one (asserted directly on
/// the HIR method table by the `class::init` unit tests; here the
/// end-to-end behavior).
#[test]
fn an_init_less_chain_instantiates_through_the_inherited_constructor() {
    build_and_run(
        "912_chain",
        "class A:\n    pass\n\n\nclass B(A):\n    pass\n\n\nb = B()\nprint(2)\n",
        "2\n",
    );
}

/// PEP 695 generic classes monomorphize the synthesized constructor per
/// instantiated type argument.
#[test]
fn a_generic_class_with_no_init_is_instantiable() {
    build_and_run(
        "912_generic",
        "class Box[T]:\n    def value(self) -> int:\n        return 1\n\n\nb = Box[int]()\nprint(b.value())\n",
        "1\n",
    );
}

/// A user class shadowing one of the seven synthetic builtin exception names
/// (D-188): the whole module is seeded with no synthetic exception classes,
/// so this is an ordinary class and gets the ordinary synthesized
/// constructor.
#[test]
fn an_init_less_class_shadowing_a_builtin_exception_name_is_instantiable() {
    build_and_run(
        "912_shadow",
        "class ValueError:\n    def f(self) -> int:\n        return 7\n\n\nv = ValueError()\nprint(v.f())\n",
        "7\n",
    );
}

/// Instantiating an abstract class stays rejected: `ensure_init` gives it a
/// constructor, but the D-380 abstract guard in `resolve_instantiation` runs
/// ahead of constructor resolution and is unaffected.
#[test]
fn an_abstract_class_with_no_init_still_cannot_be_instantiated() {
    build_fails_with(
        "912_abstract",
        "from abc import ABC, abstractmethod\n\n\nclass Shape(ABC):\n    @abstractmethod\n    def area(self) -> int:\n        ...\n\n\ns = Shape()\nprint(1)\n",
        "cannot instantiate abstract class `Shape`",
    );
}

/// #966 flipped this case, and the flip is the point of the test.
///
/// A raisable class whose MRO reaches a *user-declared* ancestor carrying
/// only the D-225 implicit constructor is now **raisable**, matching
/// CPython, which raises this exact program without complaint. Before #966
/// it was rejected `C0001` purely because `Base` carried that implicit
/// stub: `reject_own_constructor` took the first `__init__` along the MRO
/// and found `Base`'s rather than `Exception`'s. Constructor resolution now
/// ranks an implicit `object`-style constructor last everywhere it ranks
/// constructors, so `Exception.__init__` is the one found, and D-189 rule 4
/// is satisfied.
///
/// Asserted on runtime evidence rather than a bare exit code: the built
/// program must actually raise, rendering `MyError: boom`.
#[test]
fn a_raisable_class_over_a_synthesized_ancestor_constructor_now_raises() {
    const CLASSES: &str =
        "class Base:\n    pass\n\n\nclass MyError(Base, Exception):\n    pass\n\n\n";

    let dir = ScratchDir::new("912_raisable").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "prog.py",
        &format!("{CLASSES}raise MyError(\"boom\")\n"),
    );
    let out = dir.join("prog");
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "#966: the raise now compiles, got: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&out).output().unwrap();
    assert!(
        !run.status.success(),
        "an uncaught raise must not exit successfully"
    );
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains("MyError: boom"),
        "the program must actually raise `MyError`, got stderr: {stderr}"
    );

    // The identical declarations without the `raise` still compile, and
    // `Base` -- the class carrying the implicit constructor -- is still
    // ordinarily instantiable.
    build_and_run(
        "912_raisable_ok",
        &format!("{CLASSES}b = Base()\nprint(5)\n"),
        "5\n",
    );

    // #966's second, consequential flip: with `Exception.__init__` now the
    // resolved constructor, binding the class as a *value* reaches the
    // D-188 synthetic-owner guard in `resolve_instantiation` and is
    // rejected -- the same `C0001` every other raisable class already gets,
    // where before #966 this one shape slipped through and compiled.
    build_fails_with(
        "912_raisable_value",
        &format!("{CLASSES}m = MyError()\nprint(5)\n"),
        "cannot instantiate exception class `MyError` as a value",
    );
}

/// The synthesized constructor takes only `self`, so passing an argument is
/// an ordinary arity error. Asserted on the message: the emitter renders
/// this diagnostic with a zero span.
#[test]
fn calling_a_synthesized_constructor_with_an_argument_is_an_arity_error() {
    build_fails_with(
        "912_arity",
        "class Config:\n    MAX: int = 10\n\n\nc = Config(1)\nprint(c.MAX)\n",
        "`Config` expects 0 argument(s), got 1",
    );
}
