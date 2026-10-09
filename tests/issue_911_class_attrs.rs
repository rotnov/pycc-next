//! Issue #911 (Part 1 of #885): `typing.ClassVar` registration and annotated
//! scalar class attributes, end to end through the public `pycc` CLI.
//!
//! A class attribute is a **compile-time constant**: it occupies no instance
//! slot, has no runtime storage, and a read of it -- through the class name
//! (`W.MIN_WIDTH`) or through an instance (`w.MIN_WIDTH`) -- is folded to its
//! literal at MIR-lowering time. Since #960 an instance read folds only when
//! no instance slot of that name exists anywhere in the MRO; these fixtures
//! declare no such collision, so every read here folds. These tests pin both the accepted
//! surface and every rejection that keeps that model honest, including the
//! #585 scalar-only invariant.

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

/// Builds and runs `source`, asserting the program's stdout.
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

/// Asserts that `pycc check` rejects `source` with a diagnostic whose text
/// contains `needle`.
fn assert_rejected(tag: &str, source: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "main.py", source);
    let out = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let rendered = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "pycc check should reject {tag}, but it succeeded"
    );
    assert!(
        rendered.contains(needle),
        "diagnostic for {tag} should contain {needle:?}, got:\n{rendered}"
    );
}

const WINDOW: &str = "\
from typing import ClassVar


class Window:
    MIN_WIDTH: int = -1024
    MAX_WIDTH: ClassVar[int] = 4096
    KIND: str = \"window\"
    SCALE: float = 1.5
    DEBUG: bool = False

    def __init__(self, width: int) -> None:
        self.width = width

    def clamped(self) -> int:
        if self.width < Window.MIN_WIDTH:
            return Window.MIN_WIDTH
        return self.width


w = Window(10)
print(Window.MIN_WIDTH)
print(w.MAX_WIDTH)
print(w.KIND)
print(w.SCALE)
print(w.DEBUG)
print(w.clamped())
";

/// The motivating #885 snippet: reads through the class name and through an
/// instance both fold to the declared constant, and `-1024` (which parses as
/// `UnaryOp(USub, NumberLiteral(1024))`, not as a literal) is accepted.
#[test]
fn window_class_attributes_read_through_class_and_instance() {
    assert_runs(
        "911_window",
        WINDOW,
        "-1024\n4096\nwindow\n1.5\nFalse\n10\n",
    );
}

/// PEP 593: `Annotated[int, ...]` is transparent, so it is accepted as a
/// class-attribute annotation exactly as it is anywhere else.
#[test]
fn an_annotated_class_attribute_is_accepted() {
    assert_runs(
        "911_annotated",
        "from typing import Annotated\n\n\nclass C:\n    X: Annotated[int, \"units\"] = 7\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nc = C()\nprint(c.X)\n",
        "7\n",
    );
}

/// A scalar type alias resolves through the alias table like any other
/// annotation.
#[test]
fn a_type_alias_class_attribute_is_accepted() {
    assert_runs(
        "911_alias",
        "type Meters = int\n\n\nclass C:\n    X: Meters = 3\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nc = C()\nprint(c.X)\n",
        "3\n",
    );
}

/// A unary `+` on a numeric literal is accepted alongside unary `-`.
#[test]
fn a_unary_plus_class_attribute_is_accepted() {
    assert_runs(
        "911_uadd",
        "class C:\n    X: int = +5\n    Y: float = -0.5\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nc = C()\nprint(c.X)\nprint(c.Y)\n",
        "5\n-0.5\n",
    );
}

/// An `int` literal under a `float` annotation widens, matching Python's own
/// numeric tower.
#[test]
fn an_int_literal_widens_under_a_float_annotation() {
    assert_runs(
        "911_widen",
        "class C:\n    X: float = 2\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nc = C()\nprint(c.X)\n",
        "2.0\n",
    );
}

/// PEP 695 (#911 work item 8): `pycc_types::monomorphize` must carry
/// `class_attrs` through, or every class attribute silently disappears from a
/// monomorphized generic class.
#[test]
fn a_generic_class_carries_its_class_attributes_through_monomorphization() {
    assert_runs(
        "911_generic",
        "class Box[T]:\n    LIMIT: int = 8\n\n    def __init__(self, item: T) -> None:\n        self.item = item\n\n\nb = Box[int](1)\nprint(b.LIMIT)\nprint(b.item)\n",
        "8\n1\n",
    );
}

/// A class attribute must not consume an instance slot: the instance
/// attributes declared in `__init__` keep their original slot indices and
/// values regardless of how many class attributes precede them.
#[test]
fn a_class_attribute_consumes_no_instance_slot() {
    assert_runs(
        "911_no_slot",
        "class P:\n    A: int = 1\n    B: int = 2\n    C: int = 3\n\n    def __init__(self, x: int, y: int) -> None:\n        self.x = x\n        self.y = y\n\n\np = P(10, 20)\nprint(p.x)\nprint(p.y)\nprint(p.A)\n",
        "10\n20\n1\n",
    );
}

/// A class attribute declared in a base class is readable through a derived
/// class's instance, and through the derived class name.
#[test]
fn a_class_attribute_is_inherited_through_the_mro() {
    assert_runs(
        "911_inherit",
        "class Base:\n    LIMIT: int = 42\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 1\n\n\nd = Derived()\nprint(d.LIMIT)\nprint(Base.LIMIT)\n",
        "42\n42\n",
    );
}

/// #911: a *derived* class that declares its own class attribute walks the
/// whole MRO looking for a collision and finds none -- the "this base is
/// clean, keep walking" path through `reject_class_attr_collisions`, which
/// the collision tests below never reach because each of them returns on the
/// first base it inspects.
#[test]
fn a_derived_class_attribute_that_collides_with_nothing_in_the_mro_is_accepted() {
    assert_runs(
        "911_inherit_no_collision",
        "class Base:\n    def __init__(self) -> None:\n        self.n = 3\n\n    @property\n    def twice(self) -> int:\n        return self.n * 2\n\n\nclass Derived(Base):\n    LIMIT: int = 7\n\n    def __init__(self) -> None:\n        self.n = 5\n\n\nd = Derived()\nprint(Derived.LIMIT)\nprint(d.twice)\n",
        "7\n10\n",
    );
}

// -- `ClassVar` registration and placement --------------------------------

/// #911 work item 1: `from typing import ClassVar` no longer fails with
/// `C0002`. (The `WINDOW` fixture above exercises the success path; this
/// pins the registration itself against an unrelated rejection.)
#[test]
fn importing_class_var_from_typing_resolves() {
    assert_rejected(
        "911_classvar_value",
        "from typing import ClassVar\n\nx = ClassVar\n",
        "ClassVar",
    );
}

#[test]
fn a_bare_class_var_annotation_is_rejected() {
    assert_rejected(
        "911_bare_classvar",
        "class C:\n    X: ClassVar = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "a bare `ClassVar` is not a valid annotation",
    );
}

#[test]
fn a_multi_argument_class_var_is_rejected() {
    assert_rejected(
        "911_classvar_arity",
        "class C:\n    X: ClassVar[int, str] = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "ClassVar takes exactly one type argument",
    );
}

#[test]
fn class_var_in_a_parameter_annotation_is_rejected() {
    assert_rejected(
        "911_classvar_param",
        "def f(x: ClassVar[int]) -> int:\n    return 1\n",
        "only valid on a class-body attribute declaration",
    );
}

#[test]
fn a_bare_class_var_in_a_parameter_annotation_is_rejected() {
    assert_rejected(
        "911_classvar_param_bare",
        "def f(x: ClassVar) -> int:\n    return 1\n",
        "only valid on a class-body attribute declaration",
    );
}

// -- annotation-form rejections -------------------------------------------

#[test]
fn a_non_scalar_class_attribute_annotation_is_rejected() {
    assert_rejected(
        "911_non_scalar",
        "class C:\n    X: None = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "which is not a scalar slot type",
    );
}

/// #585/D-213: the scalar restriction is what keeps `__set_name__`
/// untriggerable -- a descriptor-valued class attribute is not a scalar.
#[test]
fn a_descriptor_valued_class_attribute_is_rejected() {
    assert_rejected(
        "911_descriptor",
        "class Descriptor:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass C:\n    X: Descriptor = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "which is not a scalar slot type",
    );
}

#[test]
fn a_type_parameter_class_attribute_annotation_is_rejected() {
    assert_rejected(
        "911_param_annotation",
        "class Box[T]:\n    X: T = 1\n\n    def __init__(self, item: T) -> None:\n        self.item = item\n",
        "has no constant value to fold",
    );
}

#[test]
fn an_unresolvable_class_attribute_annotation_propagates_its_error() {
    assert_rejected(
        "911_unknown_annotation",
        "class C:\n    X: Nope = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "C0001",
    );
}

#[test]
fn a_non_name_class_attribute_target_is_rejected() {
    assert_rejected(
        "911_bad_target",
        "class C:\n    o.X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "must target a bare name",
    );
}

// -- right-hand-side rejections -------------------------------------------

#[test]
fn a_value_less_annotation_is_an_instance_declaration_its_init_must_establish() {
    // #1266: a value-less, non-`ClassVar` annotation is an instance
    // attribute declaration, not a valueless class constant, so the refusal
    // names the missing `__init__` assignment instead.
    assert_rejected(
        "911_no_value",
        "class C:\n    X: int\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "instance attribute `X` declared in class `C` is never assigned at the top level of \
         `C.__init__`",
    );
}

#[test]
fn a_class_var_without_a_value_is_rejected() {
    assert_rejected(
        "911_class_var_no_value",
        "from typing import ClassVar\n\n\nclass C:\n    X: ClassVar[int]\n\n    \
         def __init__(self) -> None:\n        self.n = 0\n",
        "has no value",
    );
}

#[test]
fn a_non_literal_class_attribute_value_is_rejected() {
    assert_rejected(
        "911_call_rhs",
        "def f() -> int:\n    return 1\n\n\nclass C:\n    X: int = f()\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        // #1284 names the shape: a call is its own refusal now.
        "is initialized with a call, which is not supported yet (#1348)",
    );
}

#[test]
fn a_non_sign_unary_class_attribute_value_is_rejected() {
    assert_rejected(
        "911_not_rhs",
        "class C:\n    X: bool = not True\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "must be initialized with a literal",
    );
}

#[test]
fn a_sign_on_a_non_numeric_class_attribute_value_is_rejected() {
    assert_rejected(
        "911_signed_str",
        "class C:\n    X: str = -\"a\"\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "must be initialized with a literal",
    );
}

#[test]
fn a_complex_class_attribute_value_is_rejected() {
    assert_rejected(
        "911_complex",
        "class C:\n    X: float = 1j\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "must be initialized with a literal",
    );
}

#[test]
fn an_out_of_i64_range_class_attribute_value_is_rejected() {
    assert_rejected(
        "911_huge",
        "class C:\n    X: int = 99999999999999999999999999\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "does not fit in i64",
    );
}

#[test]
fn an_int_literal_under_a_str_annotation_is_rejected() {
    assert_rejected(
        "911_int_str",
        "class C:\n    X: str = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "is initialized with a `int` literal",
    );
}

#[test]
fn a_float_literal_under_an_int_annotation_is_rejected() {
    assert_rejected(
        "911_float_int",
        "class C:\n    X: int = 1.5\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "is initialized with a `float` literal",
    );
}

#[test]
fn a_bool_literal_under_an_int_annotation_is_rejected() {
    assert_rejected(
        "911_bool_int",
        "class C:\n    X: int = True\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "is initialized with a `bool` literal",
    );
}

#[test]
fn a_str_literal_under_an_int_annotation_is_rejected() {
    assert_rejected(
        "911_str_int",
        "class C:\n    X: int = \"a\"\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "is initialized with a `str` literal",
    );
}

// -- collisions ------------------------------------------------------------

#[test]
fn a_duplicate_class_attribute_name_is_rejected() {
    assert_rejected(
        "911_dup",
        "class C:\n    X: int = 1\n    X: int = 2\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "is already defined in class",
    );
}

/// The class attribute is declared **before** `__init__`, so a check at the
/// `AnnAssign` statement site would see an empty attribute table and let it
/// through. The reconciliation runs after the whole body walk instead.
#[test]
fn a_class_attribute_colliding_with_a_later_instance_slot_is_rejected() {
    assert_rejected(
        "911_collide_before",
        "class C:\n    x: int = 1\n\n    def __init__(self) -> None:\n        self.x = 2\n",
        "collides with an instance attribute",
    );
}

/// The same collision in the other declaration order.
#[test]
fn a_class_attribute_colliding_with_an_earlier_instance_slot_is_rejected() {
    assert_rejected(
        "911_collide_after",
        "class C:\n    def __init__(self) -> None:\n        self.x = 2\n\n    x: int = 1\n",
        "collides with an instance attribute",
    );
}

#[test]
fn a_class_attribute_colliding_with_a_property_is_rejected() {
    assert_rejected(
        "911_collide_property",
        "class C:\n    x: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n    @property\n    def x(self) -> int:\n        return self.n\n",
        "collides with an `@property`",
    );
}

#[test]
fn a_class_attribute_colliding_with_an_inherited_instance_slot_is_rejected() {
    assert_rejected(
        "911_collide_base_attr",
        "class Base:\n    def __init__(self) -> None:\n        self.x = 1\n\n\nclass Derived(Base):\n    x: int = 2\n\n    def __init__(self) -> None:\n        self.y = 3\n",
        "instance attribute inherited from `Base`",
    );
}

#[test]
fn a_class_attribute_colliding_with_an_inherited_property_is_rejected() {
    assert_rejected(
        "911_collide_base_prop",
        "class Base:\n    def __init__(self) -> None:\n        self.n = 1\n\n    @property\n    def x(self) -> int:\n        return self.n\n\n\nclass Derived(Base):\n    x: int = 2\n\n    def __init__(self) -> None:\n        self.n = 3\n",
        "`@property` inherited from `Base`",
    );
}

// -- write paths -----------------------------------------------------------

#[test]
fn writing_a_class_attribute_through_an_instance_is_rejected() {
    assert_rejected(
        "911_write_instance",
        "class C:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nc = C()\nc.X = 5\n",
        "it is a class-level attribute of class `C`",
    );
}

/// A write inside `__init__` must not be allowed to establish a colliding
/// instance slot through `collect_init_attrs`.
#[test]
fn writing_a_class_attribute_inside_init_is_rejected() {
    assert_rejected(
        "911_write_init",
        "class C:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.X = 5\n",
        "collides with an instance attribute",
    );
}

/// A write to `self.<class attr>` outside `__init__` reaches the type
/// checker's own write path rather than the lowering-time collision check.
#[test]
fn writing_a_class_attribute_through_self_outside_init_is_rejected() {
    assert_rejected(
        "911_write_self",
        "class C:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n    def bump(self) -> None:\n        self.X = 5\n",
        "it is a class-level attribute of class `C`",
    );
}

/// `C.X = 5` -- a write through the class name. Re-measured *after* the read
/// interception landed; this pins whatever diagnostic it now produces.
#[test]
fn writing_a_class_attribute_through_the_class_name_is_rejected() {
    assert_rejected(
        "911_write_class",
        "class C:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nC.X = 5\n",
        "T0021",
    );
}

// -- deliberately unsupported read shapes ---------------------------------

/// A class-attribute read folds to a constant and discards its base, so the
/// base is restricted to a plain name. A call-shaped base would otherwise
/// have its call silently dropped.
#[test]
fn reading_a_class_attribute_off_a_call_result_is_rejected() {
    assert_rejected(
        "911_call_base",
        "class C:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\ndef make() -> C:\n    return C()\n\n\nprint(make().X)\n",
        "can only be read through a plain name",
    );
}

/// #587/#433/#915: `super().X` *does* reach a base class's class attribute --
/// a class attribute is a genuine entry in the base's `__dict__`, which a
/// CPython `super` object proxies. This was out of scope for Part 1 and
/// rejected with `T0044`; #915 resolves it, and `tests/issue_915_super_class_attr.rs`
/// carries the full surface.
#[test]
fn reading_a_class_attribute_through_super_folds_to_the_base_literal() {
    assert_runs(
        "911_super",
        "class Base:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().X\n\n\nprint(Derived().read())\n",
        "1\n",
    );
}

/// PEP 634: a class attribute is a compile-time constant with no per-instance
/// value, so it cannot be a class-pattern keyword sub-pattern.
#[test]
fn a_class_attribute_in_a_class_pattern_keyword_is_rejected() {
    assert_rejected(
        "911_class_pattern",
        "class C:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nc = C()\nmatch c:\n    case C(X=v):\n        print(v)\n    case _:\n        print(0)\n",
        "not an instance attribute",
    );
}

/// PEP 544: a class attribute **does** satisfy a protocol attribute member.
/// Part 1 (#911) deliberately left `check_protocol_conformance` untouched and
/// this test pinned the resulting rejection; [#914] lifted the limitation, so
/// the same program now compiles and prints the folded constant. The full
/// surface (inheritance, mixed members, `isinstance` agreement, and the
/// type-mismatch rejection) lives in `tests/issue_914_protocol_class_attr.rs`.
#[test]
fn a_class_attribute_satisfies_a_protocol_attribute_member() {
    assert_runs(
        "911_protocol",
        "from typing import Protocol\n\n\nclass HasLimit(Protocol):\n    limit: int\n\n\nclass C:\n    limit: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\ndef read(p: HasLimit) -> int:\n    return p.limit\n\n\nprint(read(C()))\n",
        "1\n",
    );
}

/// An unknown attribute on a class that *does* declare class attributes
/// still reports `T0044` (the class-attribute lookup's miss path).
#[test]
fn an_unknown_attribute_on_a_class_with_class_attributes_is_still_t0044() {
    assert_rejected(
        "911_unknown_attr",
        "class C:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nprint(C().nope)\n",
        "T0044",
    );
}

/// The same miss, through the class name rather than an instance.
#[test]
fn an_unknown_class_name_attribute_is_still_t0044() {
    assert_rejected(
        "911_unknown_class_attr",
        "class C:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nprint(C.nope)\n",
        "T0044",
    );
}
