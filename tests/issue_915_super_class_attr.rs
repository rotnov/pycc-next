//! Issue #915: `super().CLASS_CONST` reaches a base class's class attribute,
//! end to end through the public `pycc` CLI.
//!
//! A class attribute (#911) is a genuine entry in its class's `__dict__`, so a
//! CPython `super` object proxies it along the MRO exactly as it proxies a
//! `@property`. Resolution starts at `mro[current_pos + 1..]` (#433/D-160), and
//! the read folds to its literal at MIR-lowering time, so `super().X` produces
//! MIR identical to `Base.X`. Every expected value below was taken from CPython
//! itself, not derived.

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

/// Builds and runs `source`, asserting the compiled program's stdout. A silent
/// exit 0 from `pycc check` proves nothing about the folded value, so every
/// accepted case here is executed.
fn assert_runs(tag: &str, source: &str, expected_stdout: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "main.py", source);
    let out = dir.join("main.bin");
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "pycc build should succeed for {tag}:\n{}",
        String::from_utf8_lossy(&build.stdout)
    );
    let run = Command::new(&out).output().unwrap();
    assert!(run.status.success(), "compiled program {tag} should exit 0");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        expected_stdout,
        "stdout for {tag}"
    );
}

/// Asserts that `pycc check` rejects `source` with diagnostic `code`, and that
/// the rendered text contains `needle`. Assertions are on the code and a
/// message substring, never on a rendered path, so Windows CI agrees.
fn assert_rejected(tag: &str, source: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "main.py", source);
    let out = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let rendered = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "pycc check should reject {tag}:\n{rendered}"
    );
    assert!(
        rendered.contains(&format!("error[{code}]")),
        "expected {code} for {tag}, got:\n{rendered}"
    );
    assert!(
        rendered.contains(needle),
        "expected {needle:?} for {tag}, got:\n{rendered}"
    );
}

/// The issue's own program. CPython prints `1`.
#[test]
fn super_reads_a_base_class_attribute() {
    assert_runs(
        "915_basic",
        "class Base:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().X\n\n\nprint(Derived().read())\n",
        "1\n",
    );
}

/// The lookup walks the *current class's* post-current MRO slice, so a class
/// attribute on the second base of a diamond is reachable. Walking the first
/// base's own MRO instead would silently skip `C`. CPython prints `42`.
#[test]
fn super_reads_a_class_attribute_on_the_second_base_of_a_diamond() {
    assert_runs(
        "915_diamond",
        "class A:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass B(A):\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass C(A):\n    Y: int = 42\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass D(B, C):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().Y\n\n\nprint(D().read())\n",
        "42\n",
    );
}

/// An override: `super().X` yields the base's value and `self.X` the derived
/// class's, in the same program. CPython prints `1` then `2`.
#[test]
fn super_reads_the_base_value_when_the_derived_class_overrides_it() {
    assert_runs(
        "915_override",
        "class Base:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    X: int = 2\n\n    def __init__(self) -> None:\n        self.n = 0\n\n    def sup(self) -> int:\n        return super().X\n\n    def own(self) -> int:\n        return self.X\n\n\nd = Derived()\nprint(d.sup())\nprint(d.own())\n",
        "1\n2\n",
    );
}

/// Behaviour change: a `super` object never sees the instance `__dict__`, so
/// an instance attribute contributed by one MRO branch must not mask a class
/// attribute contributed by another. Before #915 pycc rejected this with
/// `T0047`; CPython prints `1`.
///
/// **Deliberately narrowed by #969** ([D-234]): #915's `super()` resolution
/// is unchanged and still prefers `B.X` here, but the class shape itself no
/// longer reaches it. `D`'s flat layout is `[X, n]` while `B`'s own is `[n]`,
/// so `B`'s methods -- lowered once against `B`'s layout under D-154 -- would
/// address the wrong slot on a `D` instance, and the #969 gate rejects the
/// class at HIR lowering with `C0001`. This particular program happened to
/// print the right answer because nothing on the mis-addressed path ran; the
/// gate is a name-sequence prefix test on the layout, not a reachability
/// analysis, because `super()` can re-enter a shadowed base member that does
/// read a mis-addressed slot (see `tests/issue_969_mi_slot_layout.rs`). The
/// accepted narrowing is recorded in D-234 alongside the redesign that would
/// lift it.
///
/// [D-234]: ../docs/decisions/D-234-reject-multiple-inheritance-whose-base-layouts-are.md
#[test]
fn a_sibling_bases_instance_attribute_is_now_rejected_by_the_969_layout_gate() {
    assert_rejected(
        "915_mixed_diamond",
        "class B:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass C:\n    def __init__(self) -> None:\n        self.X = 5\n\n\nclass D(B, C):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().X\n\n\nprint(D().read())\n",
        "C0001",
        "instance layout is not a prefix of",
    );
}

/// #587 is unchanged where no class attribute exists anywhere in the slice:
/// a genuine instance attribute reached through `super()` is still `T0047`.
#[test]
fn super_reading_a_plain_instance_attribute_is_still_t0047() {
    assert_rejected(
        "915_t0047",
        "class Base:\n    def __init__(self) -> None:\n        self.x = 1\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().x\n\n\nprint(Derived().read())\n",
        "T0047",
        "is not readable",
    );
}

/// A name declared nowhere in the MRO is still `T0044`.
#[test]
fn super_reading_a_missing_name_is_still_t0044() {
    assert_rejected(
        "915_t0044_missing",
        "class Base:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().nope\n\n\nprint(Derived().read())\n",
        "T0044",
        "has no attribute named `nope`",
    );
}

/// The slice starts *after* the current class: a class attribute declared only
/// on the current class is not reachable through `super()`, even though
/// `self.X` and `Derived.X` both are. CPython raises
/// `AttributeError: 'super' object has no attribute 'X'` here.
#[test]
fn a_class_attribute_declared_only_on_the_current_class_is_still_t0044() {
    assert_rejected(
        "915_t0044_current_only",
        "class Base:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    X: int = 7\n\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().X\n\n\nprint(Derived().read())\n",
        "T0044",
        "has no attribute named `X`",
    );
}

/// #915: a `@classmethod` binds `cls`, not `self`, so a zero-arg `super()`
/// there has no receiver for the lowering to bind. This program was a `T0044`
/// before #915 purely because the class-attribute arm did not exist; adding
/// that arm would have turned it into a compiler abort, so the form is
/// rejected as a capability gap instead.
#[test]
fn super_class_attribute_read_inside_a_classmethod_is_c0001() {
    assert_rejected(
        "915_classmethod_attr",
        "class Base:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    @classmethod\n    def read(cls) -> int:\n        return super().X\n\n\nprint(Derived.read())\n",
        "C0001",
        "no `self` receiver to bind",
    );
}

/// The same guard covers the pre-existing method-call form, which aborted the
/// compiler before #915.
#[test]
fn super_method_call_inside_a_classmethod_is_c0001() {
    assert_rejected(
        "915_classmethod_call",
        "class Base:\n    def __init__(self) -> None:\n        self.n = 0\n\n    def m(self) -> int:\n        return 1\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    @classmethod\n    def read(cls) -> int:\n        return super().m()\n\n\nprint(Derived.read())\n",
        "C0001",
        "no `self` receiver to bind",
    );
}

/// And the `@staticmethod` property form, which also aborted the compiler.
#[test]
fn super_property_read_inside_a_staticmethod_is_c0001() {
    assert_rejected(
        "915_staticmethod_property",
        "class Base:\n    def __init__(self) -> None:\n        self.n = 0\n\n    @property\n    def p(self) -> int:\n        return 1\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    @staticmethod\n    def read() -> int:\n        return super().p\n\n\nprint(Derived.read())\n",
        "C0001",
        "no `self` receiver to bind",
    );
}

/// The slice is walked one class at a time, checking every class-level member
/// kind on that class before moving to the next -- a `super` object resolves
/// against one class `__dict__` at a time, so MRO *position* decides, not
/// member kind. Here `B` (earlier in `D`'s MRO) contributes the class
/// attribute and `C` (later) a `@property` of the same name; CPython prints
/// `1`, the class attribute. Scanning all properties first would print `99`.
#[test]
fn an_earlier_class_attribute_outranks_a_later_property_of_the_same_name() {
    assert_runs(
        "915_attr_beats_later_property",
        "class A:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass B(A):\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass C(A):\n    def __init__(self) -> None:\n        self.n = 0\n\n    @property\n    def X(self) -> int:\n        return 99\n\n\nclass D(B, C):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().X\n\n\nprint(D().read())\n",
        "1\n",
    );
}

/// The converse ordering: the `@property` is on the earlier MRO entry, so it
/// wins and its getter is called. CPython prints `99`.
#[test]
fn an_earlier_property_outranks_a_later_class_attribute_of_the_same_name() {
    assert_runs(
        "915_property_beats_later_attr",
        "class A:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass B(A):\n    def __init__(self) -> None:\n        self.n = 0\n\n    @property\n    def X(self) -> int:\n        return 99\n\n\nclass C(A):\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass D(B, C):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().X\n\n\nprint(D().read())\n",
        "99\n",
    );
}
