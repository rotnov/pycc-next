//! End-to-end proof for `.append()`, `.pop()` and `.get(k, default)` on an
//! instance-attribute receiver ([#1263], Part 2 of [#1218]).
//!
//! `docs/TYPE_SYSTEM.md`'s class "Current state" paragraph is the contract.
//! The byte-exact oracle fixture is `tests/fixtures/attr_container_methods.py`
//! (registered in `tests/conformance/classes.rs`, pinned-oracle only); this
//! file runs the same fixture against its recorded CPython 3.14.7 output on
//! every CI job, adds programs checked against whatever `python3` is
//! available, and owns the refusals.
//!
//! [#1263]: https://github.com/rotnov/pycc/issues/1263
//! [#1218]: https://github.com/rotnov/pycc/issues/1218

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn rendered(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.replace("\r\n", "\n")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

/// Builds `source` into `dir/app`, runs it, and returns its standard output.
fn build_and_run(dir: &Path, source: &Path) -> String {
    let build = pycc()
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = Command::new(dir.join("app"))
        .output()
        .expect("the program should spawn");
    assert!(run.status.success(), "{}", rendered(&run));
    stdout(&run)
}

/// Builds and runs `program`, asserts CPython prints the same, and returns
/// pycc's output.
fn matches_cpython(category: &str, program: &str) -> String {
    let dir = ScratchDir::new(category).expect("scratch");
    let source = dir.join("a.py");
    std::fs::write(&source, program).expect("write the subject");
    let pycc_out = build_and_run(&dir, &source);
    let oracle = python()
        .arg("a.py")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert_eq!(pycc_out, stdout(&oracle));
    pycc_out
}

/// Runs `pycc check` on `source` and returns the rendered diagnostics,
/// asserting that the check failed.
fn check_fails(category: &str, source: &str) -> String {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("a.py"), source).expect("write the subject");
    let output = pycc()
        .arg("check")
        .arg(dir.join("a.py"))
        .output()
        .expect("pycc should spawn");
    assert!(!output.status.success(), "{source:?} was accepted");
    rendered(&output)
}

/// A class with a `list[int]` slot `xs`, a `dict[str, int]` slot `d` and an
/// `int` slot `n`, plus one instance `c`.
const SLOTS: &str = "class C:\n    def __init__(self, xs: list[int], d: dict[str, int], n: int) \
                     -> None:\n        self.xs = xs\n        self.d = d\n        self.n = n\n\n\n\
                     c = C([1], {\"a\": 1}, 3)\n";

/// The oracle fixture, against CPython 3.14.7's recorded output: method
/// bodies, module scope, a function mutating a global instance, aliasing,
/// receiver-before-argument evaluation order (a rebinding argument and a
/// printing property getter), a property chain, `.pop()` in value position
/// and in a comprehension, and an empty-list `IndexError` caught at module
/// scope (a function-body `try` around `return self.xs.pop()` is the
/// accepted D-173 sentinel residual D-244's 2026-09-13 amendment records,
/// closing with #1031).
#[test]
fn the_fixture_matches_its_recorded_cpython_output() {
    let dir = ScratchDir::new("e2e_1263_fixture").expect("scratch");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/attr_container_methods.py");
    assert_eq!(
        build_and_run(&dir, &fixture),
        "4 1 40 2\n3 3\n3 2\n1 -1\n3 9\n10 9\n3 0\n8\n1 7 1 100\n1 100\n0 50\n\
         items\nnoisy\n2 5\n11 3\n1 100\npop from empty list\n"
    );
}

/// A user class defining `append`, `pop` and `get` turns receiver dispatch
/// on for the module (#1188): a container slot still takes the container
/// reading, while a property returning that class and a plain local take
/// the method reading.
#[test]
fn a_class_defining_the_names_keeps_both_readings_apart() {
    let out = matches_cpython(
        "e2e_1263_dispatch",
        "class Bag:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\
         \x20   def append(self, v: int) -> None:\n        self.n = self.n + v\n\n\
         \x20   def pop(self) -> int:\n        return self.n\n\n\
         \x20   def get(self, k: str, default: int) -> int:\n        return self.n + default\n\n\n\
         class Box:\n    def __init__(self, xs: list[int], d: dict[str, int]) -> None:\n        \
         self.xs = xs\n        self.d = d\n\n    @property\n    def bag(self) -> Bag:\n        \
         return shared\n\n    def total(self) -> int:\n        self.xs.append(9)\n        \
         return self.xs.pop() + self.d.get(\"a\", 0)\n\n\n\
         shared = Bag(10)\nb = Box([1, 2], {\"a\": 5})\nb.xs.append(3)\n\
         print(len(b.xs), b.xs.pop(), b.d.get(\"a\", 0), b.d.get(\"z\", 7))\n\
         b.bag.append(5)\nprint(b.bag.pop(), b.bag.get(\"k\", 1))\n\
         o = Bag(1)\no.append(2)\nprint(o.pop(), o.get(\"x\", 3))\n\
         print(b.total(), len(b.xs))\n\
         print(Box([(n := 6)], {\"a\": n}).xs.pop(), n)\n",
    );
    assert_eq!(out, "3 3 5 7\n15 16\n3 6\n14 2\n6 6\n");
}

/// A walrus inside an attribute receiver binds its target, with and without
/// receiver dispatch in the module.
#[test]
fn a_walrus_inside_the_receiver_binds_its_target() {
    let out = matches_cpython(
        "e2e_1263_walrus",
        "class Box:\n    def __init__(self, xs: list[int], d: dict[str, int]) -> None:\n        \
         self.xs = xs\n        self.d = d\n\n\n\
         def main() -> None:\n    print(Box([(n := 6)], {\"a\": 1}).xs.pop(), n)\n    \
         Box([1], {\"q\": 2}).xs.append((m := 8))\n    \
         print(m, Box([m], {\"a\": (j := 1)}).d.get(\"a\", (k := 3)), j, k)\n\n\n\
         main()\nprint(Box([(n := 4)], {\"a\": 1}).xs.pop(), n)\n",
    );
    assert_eq!(out, "6 6\n8 1 1 3\n4 4\n");
}

/// An attribute receiver of the wrong type, and a wrong value, key or
/// default, report the same diagnostics a bare-name receiver does.
#[test]
fn a_mistyped_attribute_receiver_or_argument_is_refused() {
    for (category, call, expected) in [
        (
            "e2e_1263_int_append",
            "c.n.append(1)\n",
            "error[T0033]: `int` does not support `.append()`",
        ),
        (
            "e2e_1263_int_pop",
            "c.n.pop()\n",
            "error[T0033]: `int` does not support `.pop()`",
        ),
        (
            "e2e_1263_dict_append",
            "c.d.append(1)\n",
            "error[T0033]: `dict[str, int]` does not support `.append()`",
        ),
        (
            "e2e_1263_value",
            "c.xs.append(\"s\")\n",
            "error[T0021]: cannot append `str` to a list of `int`",
        ),
        (
            "e2e_1263_key",
            "c.d.get(1, 0)\n",
            "error[T0021]: cannot look up a `int` key in a dict of `str` keys",
        ),
        (
            "e2e_1263_default",
            "c.d.get(\"a\", \"x\")\n",
            "error[T0021]: cannot use a `str` default for a dict of `int` values",
        ),
    ] {
        let text = check_fails(category, &format!("{SLOTS}{call}"));
        assert!(text.contains(expected), "{call}: {text}");
    }
}

/// A receiver that is neither a name nor an attribute read (a call result,
/// a `pycc_std` module constant) is refused with the widened wording; `.add()` keeps its bare-name-only wording, since no
/// `set` instance slot exists.
#[test]
fn an_unsupported_receiver_is_refused() {
    let text = check_fails(
        "e2e_1263_call_receiver",
        "def f() -> list[int]:\n    return [1]\n\n\nf().pop()\n",
    );
    assert!(
        text.contains(
            "error[C0001]: `.pop()` is only supported on a name or an instance attribute so far"
        ),
        "{text}"
    );
    // A `pycc_std` module constant lowers to a qualified name, not an
    // attribute read, so it is the same refusal.
    let text = check_fails("e2e_1263_std_constant", "import math\nmath.pi.append(1)\n");
    assert!(
        text.contains(
            "error[C0001]: `.append()` is only supported on a name or an instance attribute so far"
        ),
        "{text}"
    );
    let text = check_fails("e2e_1263_add", &format!("{SLOTS}c.xs.add(1)\n"));
    assert!(
        text.contains("error[C0001]: `.add()` is only supported on a bare-name set so far"),
        "{text}"
    );
}

/// An attribute of a CPython module is a foreign object. Since #1095 a
/// container method on it is the foreign method call: the module's own
/// foreign import keeps the method reading, and the object receiver takes
/// it.
#[test]
fn a_foreign_attribute_receiver_is_the_foreign_method_call() {
    for (category, source) in [
        ("e2e_1263_gc", "import gc\ngc.garbage.append(1)\n"),
        ("e2e_1263_argv", "import sys\nsys.argv.append(\"x\")\n"),
    ] {
        let dir = ScratchDir::new(category).expect("scratch");
        std::fs::write(dir.join("a.py"), source).expect("write the subject");
        let output = pycc()
            .arg("check")
            .arg(dir.join("a.py"))
            .output()
            .expect("pycc should spawn");
        assert!(output.status.success(), "{source}: {}", rendered(&output));
    }
}

/// Since #1425 a module inherits object-receiver admission from a project
/// dependency that can hold a CPython object, so an object it imports from a
/// sibling module takes the foreign method-call reading: a container method
/// on it, or on an attribute of it, passes `pycc check` (both were `I0404`
/// before #1425).
#[test]
fn an_imported_object_s_container_methods_pass_check() {
    for (category, source) in [
        (
            "e2e_1263_cross_module_attr",
            "from dep import g\n\ng.garbage.append(1)\n",
        ),
        (
            "e2e_1263_cross_module_name",
            "from dep import g\n\ng.append(1)\n",
        ),
    ] {
        let dir = ScratchDir::new(category).expect("scratch");
        std::fs::write(dir.join("dep.py"), "import gc\n\ng = gc\n").expect("write the dependency");
        std::fs::write(dir.join("a.py"), source).expect("write the subject");
        let output = pycc()
            .arg("check")
            .arg(dir.join("a.py"))
            .output()
            .expect("pycc should spawn");
        assert!(output.status.success(), "{source}: {}", rendered(&output));
    }
}

/// Known limit: an unannotated private helper returning an attribute
/// `.pop()` or `.get()` cannot have its return type inferred from its own
/// body, exactly like the bare-name forms. Since #1420 a call to it whose
/// result an annotation constrains (`def run(self) -> int: return
/// self._take()`) supplies the type, so the limit shows only where no such
/// call exists; annotating the helper works, and a `None`-returning helper
/// needs no annotation.
#[test]
fn an_unannotated_private_helper_needs_a_return_annotation() {
    for (category, helper, name) in [
        (
            "e2e_1263_take",
            "    def _take(self):\n        return self.xs.pop()\n",
            "_take",
        ),
        (
            "e2e_1263_look",
            "    def _look(self):\n        return self.d.get(\"a\", 0)\n",
            "_look",
        ),
    ] {
        let text = check_fails(
            category,
            &format!(
                "{SLOTS}\n\nclass D(C):\n{helper}\n    def run(self) -> None:\n        \
                 print(self.{name}())\n"
            ),
        );
        assert!(
            text.contains(&format!(
                "error[T0021]: cannot infer return type of private helper `D.{name}`; add an \
                 annotation"
            )),
            "{text}"
        );
    }
    // #1420: the annotated caller's `-> int` constrains each helper.
    let out = matches_cpython(
        "e2e_1263_caller_constrained",
        "class C:\n    def __init__(self, xs: list[int], d: dict[str, int]) -> None:\n        \
         self.xs = xs\n        self.d = d\n\n\
         \x20   def _take(self):\n        return self.xs.pop()\n\n\
         \x20   def _look(self):\n        return self.d.get(\"a\", 0)\n\n\
         \x20   def run(self) -> int:\n        return self._take()\n\n\
         \x20   def peek(self) -> int:\n        return self._look()\n\n\n\
         c = C([1, 2], {\"a\": 7})\nprint(c.run(), c.peek(), len(c.xs))\n",
    );
    assert_eq!(out, "2 7 1\n");
    let out = matches_cpython(
        "e2e_1263_annotated",
        "class C:\n    def __init__(self, xs: list[int]) -> None:\n        self.xs = xs\n\n\
         \x20   def _take(self) -> int:\n        return self.xs.pop()\n\n\
         \x20   def _add(self, v: int):\n        self.xs.append(v)\n\n\
         \x20   def run(self) -> int:\n        self._add(4)\n        \
         return self._take() + len(self.xs)\n\n\nprint(C([1]).run())\n",
    );
    assert_eq!(out, "5\n");
}

/// Attribute receivers inside a generic function's body, as the argument of
/// a generic class's constructor, and around generic calls, plus a bare-name
/// `.pop()` statement in an unannotated private helper: the walkers that
/// reject, rewrite and collect generic calls and instantiations, and the
/// private-helper solver's walrus pre-pass, all reach the receiver. The
/// expected output is CPython 3.14.7's, recorded rather than re-run: the
/// `python3` of an unpinned CI job may predate PEP 695's `class Box[T]`.
#[test]
fn attribute_receivers_in_generic_code_and_private_helpers_match_cpython() {
    let dir = ScratchDir::new("e2e_1263_generic").expect("scratch");
    let source = dir.join("a.py");
    std::fs::write(
        &source,
        "class C:\n    def __init__(self, xs: list[int], d: dict[str, int]) -> None:\n        \
         self.xs = xs\n        self.d = d\n\n\n\
         class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\n\n\
         def ident[T](v: T) -> T:\n    return v\n\n\n\
         def poke[T](c: C, v: T) -> T:\n    c.xs.append(1)\n    \
         print(c.d.get(\"a\", 0), c.xs.pop())\n    return v\n\n\n\
         def _drop(xs: list[int]):\n    xs.pop()\n\n\n\
         def main() -> None:\n    c = C([ident(4)], {\"a\": ident(2)})\n    ys = [5, 6]\n    \
         _drop(ys)\n    print(poke(c, 3), c.xs.pop(), len(ys))\n    \
         c.xs.append(ident(7))\n    b = Box[int](c.xs.pop())\n    \
         print(b.v, c.d.get(\"a\", ident(0)))\n\n\nmain()\n",
    )
    .expect("write the subject");
    assert_eq!(build_and_run(&dir, &source), "2 1\n3 4 1\n7 2\n");
}
