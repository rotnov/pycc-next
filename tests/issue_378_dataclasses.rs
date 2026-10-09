// Issue #378: dataclasses (PEP 557), class decorators (PEP 3129), and
// dataclass_transform (PEP 681) integration tests.
//
// These tests exercise pycc's narrow dataclass implementation end-to-end:
// compiling a .py source with `@dataclass` classes, running the resulting
// binary, and checking stdout. Error-path tests verify that unsupported
// shapes produce C0001 diagnostics.

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

fn build_and_run(dir: &std::path::Path, src: &std::path::Path, bin_name: &str) -> (bool, Vec<u8>) {
    let out = dir.join(bin_name);
    let build_status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    if !build_status.success() {
        return (false, Vec::new());
    }
    let output = Command::new(&out).output().unwrap();
    (output.status.success(), output.stdout)
}

fn check_fails(dir: &std::path::Path, src: &std::path::Path) -> bool {
    let out = dir.join("should_not_compile");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    !status.success()
}

/// #378: a basic `@dataclass` with two int fields and auto-generated
/// `__init__` -- `Point(1, 2)` sets `x` and `y`, and `p.x`/`p.y` read
/// them back.
#[test]
fn dataclass_basic_constructor() {
    let dir = ScratchDir::new("378_basic").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "basic.py",
        "@dataclass\nclass Point:\n    x: int\n    y: int\np = Point(1, 2)\nprint(p.x)\nprint(p.y)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "basic");
    assert!(
        ok,
        "pycc build and run should succeed for a basic dataclass"
    );
    assert_eq!(stdout, b"1\n2\n", "dataclass fields should be 1 and 2");
}

/// #378: auto-generated `__repr__` produces `ClassName(field=value, ...)`.
#[test]
fn dataclass_repr() {
    let dir = ScratchDir::new("378_repr").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "repr.py",
        "@dataclass\nclass Point:\n    x: int\n    y: int\np = Point(3, 4)\nprint(p)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "repr");
    assert!(ok, "pycc build and run should succeed for dataclass repr");
    assert_eq!(
        stdout, b"Point(x=3, y=4)\n",
        "dataclass repr should be Point(x=3, y=4)"
    );
}

/// #378: auto-generated `__eq__` and `__ne__` -- same-field instances are
/// equal, different-field instances are not.
#[test]
fn dataclass_equality() {
    let dir = ScratchDir::new("378_eq").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "eq.py",
        "@dataclass\nclass Point:\n    x: int\n    y: int\np1 = Point(1, 2)\np2 = Point(1, 2)\np3 = Point(3, 4)\nprint(p1 == p2)\nprint(p1 != p3)\nprint(p1 == p3)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "eq");
    assert!(
        ok,
        "pycc build and run should succeed for dataclass equality"
    );
    assert_eq!(
        stdout, b"True\nTrue\nFalse\n",
        "dataclass equality: True, True, False"
    );
}

/// #378: dataclass inheritance merges parent fields before child fields.
#[test]
fn dataclass_inheritance() {
    let dir = ScratchDir::new("378_inh").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "inh.py",
        "@dataclass\nclass Base:\n    a: int\n@dataclass\nclass Derived(Base):\n    b: int\nd = Derived(1, 2)\nprint(d.a)\nprint(d.b)\nprint(d)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "inh");
    assert!(
        ok,
        "pycc build and run should succeed for dataclass inheritance"
    );
    assert_eq!(
        stdout, b"1\n2\nDerived(a=1, b=2)\n",
        "dataclass inheritance: parent fields first"
    );
}

/// #378: dataclass inheritance with equality -- inherited fields
/// participate in `__eq__`.
#[test]
fn dataclass_inheritance_equality() {
    let dir = ScratchDir::new("378_inh_eq").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "inh_eq.py",
        "@dataclass\nclass Base:\n    a: int\n@dataclass\nclass Derived(Base):\n    b: int\nd1 = Derived(1, 2)\nd2 = Derived(1, 2)\nd3 = Derived(1, 3)\nprint(d1 == d2)\nprint(d1 != d3)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "inh_eq");
    assert!(
        ok,
        "pycc build and run should succeed for dataclass inheritance equality"
    );
    assert_eq!(
        stdout, b"True\nTrue\n",
        "dataclass inheritance equality: True, True"
    );
}

/// #378 (PEP 681): `@dataclass_transform()` is equivalent to `@dataclass`.
#[test]
fn dataclass_transform_equivalent() {
    let dir = ScratchDir::new("378_dct").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "dct.py",
        "@dataclass_transform()\nclass Point:\n    x: int\n    y: int\np = Point(1, 2)\nprint(p)\nprint(p == Point(1, 2))\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "dct");
    assert!(
        ok,
        "pycc build and run should succeed for dataclass_transform"
    );
    assert_eq!(
        stdout, b"Point(x=1, y=2)\nTrue\n",
        "dataclass_transform should work like dataclass"
    );
}

/// #378: a non-dataclass class decorator is rejected with C0001.
#[test]
fn reject_non_dataclass_decorator() {
    let dir = ScratchDir::new("378_rej_deco").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_deco.py",
        "@some_decorator\nclass Point:\n    x: int\n",
    );
    assert!(
        check_fails(&dir, &src),
        "a non-dataclass class decorator should be rejected"
    );
}

/// #378: `@dataclass(frozen=True)` is rejected with C0001.
#[test]
fn reject_dataclass_with_options() {
    let dir = ScratchDir::new("378_rej_frozen").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_frozen.py",
        "@dataclass(frozen=True)\nclass Point:\n    x: int\n",
    );
    assert!(
        check_fails(&dir, &src),
        "`@dataclass(frozen=True)` should be rejected"
    );
}

/// #378: `field(default=...)` is rejected with C0001.
#[test]
fn reject_field_default() {
    let dir = ScratchDir::new("378_rej_field").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_field.py",
        "@dataclass\nclass Point:\n    x: int = field(default=0)\n",
    );
    assert!(
        check_fails(&dir, &src),
        "`field(default=...)` should be rejected"
    );
}

/// #378: `field(default_factory=...)` is rejected with C0001.
#[test]
fn reject_field_default_factory() {
    let dir = ScratchDir::new("378_rej_factory").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_factory.py",
        "@dataclass\nclass Point:\n    x: int = field(default_factory=int)\n",
    );
    assert!(
        check_fails(&dir, &src),
        "`field(default_factory=...)` should be rejected"
    );
}

/// #378: a plain default value (`x: int = 42`) is rejected with C0001.
#[test]
fn reject_plain_default() {
    let dir = ScratchDir::new("378_rej_plain").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_plain.py",
        "@dataclass\nclass Point:\n    x: int = 42\n",
    );
    assert!(
        check_fails(&dir, &src),
        "a plain default value should be rejected"
    );
}

/// #378: an explicit `__init__` in a `@dataclass` is rejected with C0001.
#[test]
fn reject_explicit_init() {
    let dir = ScratchDir::new("378_rej_init").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_init.py",
        "@dataclass\nclass Point:\n    x: int\n    def __init__(self, x: int) -> None:\n        self.x = x\n",
    );
    assert!(
        check_fails(&dir, &src),
        "an explicit `__init__` in a dataclass should be rejected"
    );
}

/// #378: a `@dataclass` inheriting from a non-dataclass base is rejected.
#[test]
fn reject_non_dataclass_base() {
    let dir = ScratchDir::new("378_rej_base").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_base.py",
        "class Base:\n    def __init__(self, a: int) -> None:\n        self.a = a\n@dataclass\nclass Derived(Base):\n    b: int\n",
    );
    assert!(
        check_fails(&dir, &src),
        "a dataclass inheriting from a non-dataclass should be rejected"
    );
}

/// #378: an explicit `__eq__` in a `@dataclass` is rejected with C0001.
#[test]
fn reject_explicit_eq() {
    let dir = ScratchDir::new("378_rej_eq").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_eq.py",
        "@dataclass\nclass Point:\n    x: int\n    def __eq__(self, other: Point) -> bool:\n        return self.x == other.x\n",
    );
    assert!(
        check_fails(&dir, &src),
        "an explicit `__eq__` in a dataclass should be rejected"
    );
}

/// #378: an explicit `__repr__` in a `@dataclass` is rejected with C0001.
#[test]
fn reject_explicit_repr() {
    let dir = ScratchDir::new("378_rej_repr").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_repr.py",
        "@dataclass\nclass Point:\n    x: int\n    def __repr__(self) -> str:\n        return \"Point\"\n",
    );
    assert!(
        check_fails(&dir, &src),
        "an explicit `__repr__` in a dataclass should be rejected"
    );
}

/// #378: multiple class decorators are rejected with C0001.
#[test]
fn reject_multiple_decorators() {
    let dir = ScratchDir::new("378_rej_multi").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_multi.py",
        "@dataclass\n@some_other_decorator\nclass Point:\n    x: int\n",
    );
    assert!(
        check_fails(&dir, &src),
        "unsupported class decorators should be rejected"
    );
}

/// #378: a non-name target in a dataclass field annotation is rejected.
#[test]
fn reject_non_name_field_target() {
    let dir = ScratchDir::new("378_rej_target").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_target.py",
        "@dataclass\nclass Point:\n    self.x: int\n",
    );
    assert!(
        check_fails(&dir, &src),
        "a non-name field target should be rejected"
    );
}

/// #378: a zero-field dataclass is valid -- `__init__` takes no args,
/// `__eq__` always returns True, `__repr__` returns `ClassName()`.
#[test]
fn zero_field_dataclass() {
    let dir = ScratchDir::new("378_zero").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "zero.py",
        "@dataclass\nclass Empty:\n    pass\ne1 = Empty()\ne2 = Empty()\nprint(e1 == e2)\nprint(e1)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "zero");
    assert!(
        ok,
        "pycc build and run should succeed for a zero-field dataclass"
    );
    assert_eq!(
        stdout, b"True\nEmpty()\n",
        "zero-field dataclass: True, Empty()"
    );
}

/// #378: f-string interpolation of a dataclass instance uses `__repr__`.
#[test]
fn dataclass_fstring_interpolation() {
    let dir = ScratchDir::new("378_fstr").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "fstr.py",
        "@dataclass\nclass Point:\n    x: int\n    y: int\np = Point(1, 2)\nprint(f\"{p}\")\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "fstr");
    assert!(
        ok,
        "pycc build and run should succeed for f-string interpolation"
    );
    assert_eq!(
        stdout, b"Point(x=1, y=2)\n",
        "f-string interpolation should use __repr__"
    );
}

/// #378: a float field in a dataclass works with repr and equality.
#[test]
fn dataclass_float_field() {
    let dir = ScratchDir::new("378_float").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "float.py",
        "@dataclass\nclass Measurement:\n    value: float\n    unit: int\nm1 = Measurement(3.14, 1)\nm2 = Measurement(3.14, 1)\nprint(m1 == m2)\nprint(m1)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "float");
    assert!(
        ok,
        "pycc build and run should succeed for a float-field dataclass"
    );
    assert_eq!(
        stdout, b"True\nMeasurement(value=3.14, unit=1)\n",
        "float field repr and equality"
    );
}

/// #378: ordering operators (`<`, `<=`, `>`, `>=`) between dataclass
/// instances are rejected with T0021 -- pycc has no `__lt__`/`__le__`/
/// `__gt__`/`__ge__` dispatch.
#[test]
fn reject_ordering_between_dataclass_instances() {
    let dir = ScratchDir::new("378_rej_ordering").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_ordering.py",
        "@dataclass\nclass Point:\n    x: int\np1 = Point(1)\np2 = Point(2)\nprint(p1 < p2)\n",
    );
    assert!(
        check_fails(&dir, &src),
        "ordering between dataclass instances should be rejected with T0021"
    );
}

/// #378: `==` between non-dataclass instances with a user-defined `__eq__`
/// is rejected with T0021 -- the MIR `__eq__` rewrite is restricted to
/// dataclass classes (whose synthesized `__eq__` has a known-correct
/// signature).
#[test]
fn reject_eq_between_non_dataclass_instances_with_user_eq() {
    let dir = ScratchDir::new("378_rej_nondc_eq").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_nondc_eq.py",
        "class C:\n    def __init__(self, x: int) -> None:\n        self.x = x\n    def __eq__(self, other: C) -> bool:\n        return self.x == other.x\nc1 = C(1)\nc2 = C(2)\nprint(c1 == c2)\n",
    );
    assert!(
        check_fails(&dir, &src),
        "== between non-dataclass instances with user __eq__ should be rejected with T0021"
    );
}

/// #378: a self-referential dataclass field (`next: Node`) is rejected with
/// C0001 -- the instance attribute-slot storage is a single word per slot
/// (D-154), with no representation for a class instance. Without the
/// scalar-slot-type check in HIR lowering, this panics in codegen with an
/// "internal error" message; the check rejects it cleanly at HIR time.
#[test]
fn reject_self_referential_dataclass_field() {
    let dir = ScratchDir::new("378_rej_self_ref").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_self_ref.py",
        "@dataclass\nclass Node:\n    next: Node\n",
    );
    assert!(
        check_fails(&dir, &src),
        "a self-referential dataclass field should be rejected with C0001, not panic in codegen"
    );
}

/// #378: a `Self`-typed dataclass field (`next: Self`) is rejected with
/// C0001 -- `Self` resolves to `Ty::Instance(class_name)` (PEP 673), the
/// same non-scalar case as a self-referential class-name field.
#[test]
fn reject_self_typed_dataclass_field() {
    let dir = ScratchDir::new("378_rej_self_typed").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_self_typed.py",
        "@dataclass\nclass Node:\n    next: Self\n",
    );
    assert!(
        check_fails(&dir, &src),
        "a `Self`-typed dataclass field should be rejected with C0001, not panic in codegen"
    );
}

/// #378: a `None`-typed dataclass field (`x: None`) is rejected with C0001
/// -- `None` is not a scalar slot type.
#[test]
fn reject_none_typed_dataclass_field() {
    let dir = ScratchDir::new("378_rej_none_field").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rej_none_field.py",
        "@dataclass\nclass C:\n    x: None\n",
    );
    assert!(
        check_fails(&dir, &src),
        "a `None`-typed dataclass field should be rejected with C0001"
    );
}
