//! Unit tests of `string_conversion`.

use super::*;
use crate::{check, check_function};
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt};

fn parse_lower(source: &str) -> HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    pycc_hir::lower_checked(&module).expect("test fixture must lower")
}

fn check_source(source: &str) -> Result<(), Diagnostic> {
    check(&parse_lower(source))
}

fn environment_for(source: &str) -> Environment {
    let hir = parse_lower(source);
    let mut env = Environment::new();
    crate::class::bind_classes(&mut env, &hir);
    env
}

#[track_caller]
fn assert_rejected(source: &str, fragment: &str) {
    let err = check_source(source).expect_err("source must be rejected");
    assert_eq!(err.code, "C0001", "unexpected diagnostic: {err:?}");
    assert!(
        err.message.contains(fragment),
        "message {:?} lacks {fragment:?}",
        err.message
    );
}

/// For a shape that *calls* a user class declared under a builtin
/// exception name (`ValueError(1)`): the concrete pass rejects it with
/// this module's message, which makes `check_all_keyed` fall through
/// to the solver, whose `ConstraintEnvironment` has no class table and
/// therefore classifies the same call as ``call to builtin
/// `ValueError` `` -- and under D-220's solver-first merge that
/// pre-existing message is the one `pycc check` prints. So pin the
/// pipeline's verdict by code only, and pin this module's own verdict
/// (with the real seeded/shadowed provenance from `bind_classes`)
/// directly.
#[track_caller]
fn assert_rejected_by_code_and_predicate(
    source: &str,
    class_name: &str,
    site: StringConversionSite,
    fragment: &str,
) {
    let err = check_source(source).expect_err("source must be rejected");
    assert_eq!(err.code, "C0001", "unexpected diagnostic: {err:?}");
    let env = environment_for(source);
    assert!(!env.is_synthetic_class(class_name));
    assert!(env.lookup_class(class_name).is_some());
    let ty = Ty::Instance(Box::new(class_name.to_string()));
    let err = reject_unrenderable(&env, &ty, site).expect_err("predicate must reject");
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains(fragment),
        "message {:?} lacks {fragment:?}",
        err.message
    );
}

#[track_caller]
fn assert_accepted(source: &str) {
    check_source(source).expect("source must type-check");
}

const PLAIN: &str = "class C:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n";
const DATACLASS: &str = "@dataclass\nclass P:\n    x: int\n    y: int\n\n";

// -- the issue's own shapes -----------------------------------------

#[test]
fn a_plain_instance_as_a_print_argument_is_c0001() {
    assert_rejected(
        &format!("{PLAIN}a = C(1)\nprint(a)\n"),
        "string conversion of a `C` instance as a `print()` argument",
    );
}

#[test]
fn a_plain_instance_interpolated_in_an_fstring_is_c0001() {
    assert_rejected(
        &format!("{PLAIN}a = C(1)\ns: str = f\"{{a}}\"\n"),
        "string conversion of a `C` instance as an f-string interpolation",
    );
}

#[test]
fn a_plain_instance_carries_the_dataclass_help() {
    let err = check_source(&format!("{PLAIN}print(C(1))\n")).unwrap_err();
    assert_eq!(
        err.help.as_deref(),
        Some(
            "print the instance's attributes individually, or declare `C` with \
             `@dataclass` (or give it a `__repr__(self) -> str`)"
        )
    );
}

/// Codex P2 on PR #985: `@dataclass` cannot rescue a class under a
/// builtin exception name (the predicate rejects it by name before
/// consulting its shape), so the help must not recommend it.
#[test]
fn a_class_under_a_builtin_exception_name_carries_the_rename_help() {
    let env = environment_for(
        "class ValueError:\n    def __init__(self) -> None:\n        return\n\n",
    );
    let ty = Ty::Instance(Box::new("ValueError".to_string()));
    let err = reject_unrenderable(&env, &ty, StringConversionSite::PrintArgument)
        .expect_err("predicate must reject");
    assert_eq!(
        err.help.as_deref(),
        Some(
            "print the instance's attributes individually, or rename `ValueError` so it \
             no longer shadows the builtin exception `ValueError`; adding `@dataclass` \
             does not make a class under a builtin exception name renderable"
        )
    );
}

/// The rename help also reaches a `@dataclass` under a builtin
/// exception name -- the shape D-237 records as the deliberate loss --
/// and a user class under an `OSError`-family name.
#[test]
fn a_dataclass_under_a_builtin_exception_name_carries_the_rename_help() {
    let env = environment_for("@dataclass\nclass FileNotFoundError:\n    x: int\n\n");
    let ty = Ty::Instance(Box::new("FileNotFoundError".to_string()));
    let err = reject_unrenderable(&env, &ty, StringConversionSite::FStringInterpolation)
        .expect_err("predicate must reject");
    let help = err.help.expect("help must be present");
    assert!(help.contains("rename `FileNotFoundError`"), "help: {help}");
    assert!(
        !help.contains("declare `FileNotFoundError` with"),
        "help: {help}"
    );
}

/// Codex P1 on PR #985: the origin generic class is a dataclass, so
/// `check` accepts `print(Box[int](1))`; `build` re-infers the rewritten
/// call against the `0gen_Box__T_int` specialization, which must keep
/// the dataclass identity (and the substituted field list) or the gate
/// contradicts `check`'s own verdict.
#[test]
fn a_generic_dataclass_specialization_stays_renderable_after_monomorphization() {
    let source = "@dataclass\nclass Box[T]:\n    n: int\n\nb = Box[int](1)\nprint(b)\ns: str = f\"{b}\"\n";
    assert_accepted(source);
    let resolved = crate::check_and_resolve(&parse_lower(source))
        .expect("monomorphized module must type-check");
    let specialization = resolved
        .class_defs
        .iter()
        .find_map(|(name, def)| (name == "0gen_Box__T_int").then_some(def))
        .expect("specialization must be registered");
    assert!(specialization.is_dataclass);
    assert_eq!(
        specialization.dataclass_fields,
        vec![("n".to_string(), Ty::Int)]
    );
    assert!(
        specialization
            .methods
            .iter()
            .any(|(name, _)| name == "__repr__")
    );
    let mut env = Environment::new();
    crate::class::bind_classes(&mut env, &resolved);
    let ty = Ty::Instance(Box::new("0gen_Box__T_int".to_string()));
    reject_unrenderable(&env, &ty, StringConversionSite::PrintArgument)
        .expect("specialization must be renderable");
}

/// A `T`-typed dataclass field is substituted on the specialization
/// exactly as `attrs` is.
#[test]
fn a_generic_dataclass_field_typed_by_the_parameter_is_substituted() {
    // `value: T` also synthesizes `__eq__` over `T`, which the checker
    // rejects with `T0021` before monomorphization (a pre-existing
    // limit outside D-237), so the module is hand-built instead.
    let param = Ty::Param(Box::new("T".to_string()));
    let self_ty = Ty::Instance(Box::new("Box".to_string()));
    let init = HirItem::Function {
        name: "Box.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty),
            ("value".to_string(), param.clone()),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "value".to_string(),
            value: HirExpr::Name("value".to_string()),
        }],
    };
    let class_def = pycc_hir::HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Box".to_string(),
        bases: Vec::new(),
        mro: vec!["Box".to_string()],
        attrs: vec![("value".to_string(), param.clone())],
        methods: vec![("__init__".to_string(), "Box.__init__".to_string())],
        type_param: Some("T".to_string()),
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: true,
        dataclass_fields: vec![("value".to_string(), param)],
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "Box".to_string(),
                type_arg: Ty::Str,
                args: vec![HirExpr::StringLiteral("x".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("Box".to_string(), class_def)],
    };
    let resolved = crate::check_and_resolve(&hir).expect("module must type-check");
    let specialization = resolved
        .class_defs
        .iter()
        .find_map(|(name, def)| (name == "0gen_Box__T_str").then_some(def))
        .expect("specialization must be registered");
    assert!(specialization.is_dataclass);
    assert_eq!(
        specialization.dataclass_fields,
        vec![("value".to_string(), Ty::Str)]
    );
}

const UPCAST: &str = "from typing import cast\n\n@dataclass\nclass Base:\n    x: int\n\n\
class Derived(Base):\n    pass\n\nd = Derived(1)\n";

/// Codex round 2 on PR #985: MIR erases `cast(Base, d)` to `d`, so the
/// gate judges the erased value too; a subclass inheriting the
/// dataclass `__repr__` renders through it.
#[test]
fn a_cast_from_a_plain_subclass_to_a_dataclass_base_renders_under_print() {
    assert_accepted(&format!("{UPCAST}print(cast(Base, d))\n"));
    assert_accepted(&format!("{UPCAST}s: str = f\"{{cast(Base, d)}}\"\n"));
    assert_accepted(&format!("{UPCAST}print(cast(Base, cast(Derived, d)))\n"));
}


#[test]
fn a_cast_to_the_dataclass_own_type_stays_renderable() {
    assert_accepted(&format!(
        "{DATACLASS}from typing import cast\n\np = P(1, 2)\nprint(cast(P, p))\ns: str = f\"{{cast(P, p)}}\"\n"
    ));
}

/// A user `def cast` is an ordinary function call: nothing is erased,
/// so only its declared return type is judged.
#[test]
fn a_user_defined_cast_is_not_looked_through() {
    assert_accepted("def cast(a: int, b: int) -> int:\n    return b\n\nprint(cast(1, 2))\n");
}

#[test]
fn a_class_without_an_init_is_c0001() {
    // D-225's implicit constructor: `C()` type-checks, so the instance
    // reaches the gate.
    assert_rejected("class C:\n    pass\n\nprint(C())\n", "`C` instance");
}

#[test]
fn a_user_written_repr_makes_the_instance_renderable() {
    assert_accepted(
        "class C:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n\
         \x20   def __repr__(self) -> str:\n        return \"C()\"\n\n\
         print(C(1))\nprint(str(C(1)), repr(C(1)))\n",
    );
}

#[test]
fn a_text_dunder_that_does_not_return_str_is_c0001() {
    assert_rejected(
        "class C:\n    def __repr__(self) -> int:\n        return 1\n\nprint(C())\n",
        "`C` instance",
    );
    assert_rejected(
        "class C:\n    def __str__(self, x: int) -> str:\n        return \"\"\n\nprint(C())\n",
        "`C` instance",
    );
}

#[test]
fn repr_of_an_instance_ignores_its_str() {
    let source = "class C:\n    def __str__(self) -> str:\n        return \"s\"\n\n";
    assert_accepted(&format!("{source}print(C(), str(C()))\n"));
    let err = check_source(&format!("{source}x = repr(C())\n")).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn an_enum_member_renders() {
    assert_accepted(
        "from enum import Enum\n\nclass Color(Enum):\n    RED = 1\n    BLUE = 2\n\n\
         print(Color.RED)\n",
    );
}

#[test]
fn a_non_dataclass_subclass_of_a_dataclass_inherits_its_repr() {
    assert_accepted(&format!(
        "{DATACLASS}class Q(P):\n    pass\n\nprint(Q(1, 2))\n"
    ));
}

#[test]
fn an_instance_in_a_later_print_argument_is_c0001() {
    assert_rejected(&format!("{PLAIN}print(\"x\", C(1), 1)\n"), "`C` instance");
}

#[test]
fn a_protocol_typed_parameter_is_c0001_with_the_protocol_message() {
    assert_rejected(
        "from typing import Protocol\n\nclass Shape(Protocol):\n    def area(self) -> int: ...\n\n\
         def show(x: Shape) -> None:\n    print(x)\n",
        "string conversion of a value typed as protocol `Shape` as a `print()` argument",
    );
}

#[test]
fn a_protocol_typed_parameter_interpolated_in_an_fstring_is_c0001() {
    assert_rejected(
        "from typing import Protocol\n\nclass Shape(Protocol):\n    def area(self) -> int: ...\n\n\
         def show(x: Shape) -> str:\n    return f\"{x}\"\n",
        "protocol `Shape` as an f-string interpolation",
    );
}

// -- shadowing builtin exception names (provenance, not shape) ------

#[test]
fn a_user_class_named_value_error_is_c0001() {
    assert_rejected_by_code_and_predicate(
        "class ValueError:\n    def __init__(self) -> None:\n        return\n\n\
         v = ValueError()\nprint(v)\n",
        "ValueError",
        StringConversionSite::PrintArgument,
        "`ValueError` instance as a `print()` argument",
    );
}

#[test]
fn a_dataclass_named_value_error_is_c0001() {
    assert_rejected_by_code_and_predicate(
        "@dataclass\nclass ValueError:\n    x: int\n\nprint(ValueError(1))\n",
        "ValueError",
        StringConversionSite::PrintArgument,
        "`ValueError` instance",
    );
}

#[test]
fn a_dataclass_named_exception_interpolated_in_an_fstring_is_c0001() {
    assert_rejected_by_code_and_predicate(
        "@dataclass\nclass Exception:\n    x: int\n\nv = Exception(1)\ns: str = f\"{v}\"\n",
        "Exception",
        StringConversionSite::FStringInterpolation,
        "`Exception` instance as an f-string interpolation",
    );
}

#[test]
fn a_dataclass_named_after_an_os_error_family_member_is_c0001() {
    // Renders today (`FileNotFoundError(x=3)`); rejected because the
    // name is decided before the shape -- the loss D-237 records.
    assert_rejected_by_code_and_predicate(
        "@dataclass\nclass FileNotFoundError:\n    x: int\n\nprint(FileNotFoundError(3))\n",
        "FileNotFoundError",
        StringConversionSite::PrintArgument,
        "`FileNotFoundError` instance",
    );
}

#[test]
fn a_dataclass_named_exception_group_printed_directly_is_c0001() {
    assert_rejected_by_code_and_predicate(
        "@dataclass\nclass ExceptionGroup:\n    x: int\n\nprint(ExceptionGroup(1))\n",
        "ExceptionGroup",
        StringConversionSite::PrintArgument,
        "`ExceptionGroup` instance",
    );
}

#[test]
fn a_dataclass_named_exception_group_bound_by_except_star_is_c0001() {
    assert_rejected(
        "@dataclass\nclass ExceptionGroup:\n    x: int\n\n\
         def main() -> None:\n    try:\n        raise ValueError(\"boom\")\n\
         \x20   except* ValueError as eg:\n        print(eg)\n\nmain()\n",
        "`ExceptionGroup` instance",
    );
}

#[test]
fn an_unseeded_except_star_binding_is_c0001() {
    // `class OSError:` shadows a builtin name, so lowering withholds the
    // whole seeded table; `ExceptionGroup` is not one of the flat seven,
    // so its absence has no name-based fallback in MIR.
    assert_rejected(
        "class OSError:\n    LIMIT: int = 1\n\n\
         def main() -> None:\n    try:\n        raise ValueError(\"boom\")\n\
         \x20   except* ValueError as eg:\n        print(eg)\n\nmain()\n",
        "`ExceptionGroup` instance",
    );
}

#[test]
fn a_user_exception_class_instantiated_as_a_value_is_c0001() {
    // Carries a D-189 tag; MIR's class-table fallback would rewrite the
    // plain instance to an exception message and abort.
    assert_rejected(
        "class MyErr(Exception):\n    def __init__(self) -> None:\n        return\n\n\
         e = MyErr()\nprint(e)\n",
        "`MyErr` instance",
    );
}

#[test]
fn a_user_exception_class_interpolated_in_an_fstring_is_c0001() {
    assert_rejected(
        "class MyErr(Exception):\n    def __init__(self) -> None:\n        return\n\n\
         e = MyErr()\ns: str = f\"{e}\"\n",
        "`MyErr` instance",
    );
}

#[test]
fn a_missing_class_table_entry_is_an_ordinary_rejection() {
    // Only `check_function`'s empty class table reaches the `None` side
    // of the non-builtin branch; no CLI program can.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "x".to_string(),
            Ty::Instance(Box::new("Missing".to_string())),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Name("x".to_string())],
        })],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("`Missing` instance"),
        "{}",
        err.message
    );
}

// -- accepted controls ----------------------------------------------

#[test]
fn a_dataclass_instance_renders_under_print_and_fstring() {
    assert_accepted(&format!(
        "{DATACLASS}p = P(1, 2)\nprint(p)\ns: str = f\"{{p}}\"\n"
    ));
}

#[test]
fn a_seeded_flat_seven_binding_renders() {
    assert_accepted(
        "def main() -> None:\n    try:\n        raise ValueError(\"boom\")\n\
         \x20   except ValueError as e:\n        print(e)\n\nmain()\n",
    );
}

#[test]
fn a_seeded_os_error_family_binding_renders() {
    assert_accepted(
        "def main() -> None:\n    try:\n        raise FileNotFoundError(\"gone\")\n\
         \x20   except FileNotFoundError as e:\n        print(e)\n\nmain()\n",
    );
}

#[test]
fn a_seeded_except_star_binding_renders() {
    assert_accepted(
        "def main() -> None:\n    try:\n        raise ValueError(\"boom\")\n\
         \x20   except* ValueError as eg:\n        print(eg)\n\nmain()\n",
    );
}

#[test]
fn an_unseeded_flat_seven_binding_renders() {
    // The unrelated `class OSError:` withholds seeding, so
    // `lookup_class("ValueError")` is `None`, but MIR still resolves the
    // flat seven by name.
    let source = "class OSError:\n    LIMIT: int = 1\n\n\
         def main() -> None:\n    try:\n        raise ValueError(\"boom\")\n\
         \x20   except ValueError as e:\n        print(e)\n\nmain()\n";
    let env = environment_for(source);
    assert!(env.lookup_class("ValueError").is_none());
    assert!(!env.is_synthetic_class("ValueError"));
    assert_accepted(source);
}

#[test]
fn a_dataclass_named_os_error_leaves_a_plain_flat_seven_binding_renderable() {
    assert_accepted(
        "@dataclass\nclass OSError:\n    x: int\n\n\
         def main() -> None:\n    try:\n        raise ValueError(\"boom\")\n\
         \x20   except ValueError as e:\n        print(e)\n\nmain()\n",
    );
}

#[test]
fn scalar_print_arguments_are_untouched() {
    assert_accepted("print(1, \"s\", 2.0, True)\ns: str = f\"{1} {2.0} {True}\"\n");
}

#[test]
fn each_site_has_its_own_wording() {
    let env = Environment::new();
    let ty = Ty::Instance(Box::new("C".to_string()));
    let print_err =
        reject_unrenderable(&env, &ty, StringConversionSite::PrintArgument).unwrap_err();
    assert!(print_err.message.contains("as a `print()` argument"));
    let fstring_err =
        reject_unrenderable(&env, &ty, StringConversionSite::FStringInterpolation).unwrap_err();
    assert!(fstring_err.message.contains("as an f-string interpolation"));
    let protocol = Ty::Protocol(Box::new("Shape".to_string()));
    let protocol_err =
        reject_unrenderable(&env, &protocol, StringConversionSite::FStringInterpolation)
            .unwrap_err();
    assert!(
        protocol_err
            .message
            .contains("protocol `Shape` as an f-string interpolation")
    );
    assert!(protocol_err.help.is_none());
    assert!(reject_unrenderable(&env, &Ty::Int, StringConversionSite::PrintArgument).is_ok());
}
