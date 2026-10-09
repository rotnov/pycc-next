//! Issue #977 (D-237): the string-conversion gate for `print` arguments and
//! f-string interpolations.
//!
//! `pycc_codegen`'s `to_str` renders scalars (`int`, `float`, `bool`, `str`,
//! `None`) directly and panics on a `Scalar::Instance`. The only way an
//! instance-typed value avoids that panic is a MIR rewrite: either
//! `rewrite_instance_to_repr` (a `@dataclass` instance becomes a call to its
//! synthesized `__repr__`) or `rewrite_exception_to_message` (a caught
//! builtin exception becomes `MirExpr::ExceptionMessage`). Until #977 the
//! type checker placed no restriction on either surface, so
//! `class C: ...; print(C())` passed `pycc check` and panicked in `pycc
//! build`, and several shadowing shapes passed `check` and aborted at
//! runtime. This module is the fail-closed predicate that keeps every such
//! program out of the backend: it is a conservative *under-approximation* of
//! what MIR renders, decided by **name provenance first, shape second**, and
//! it emits `C0001` (a capability gap, not a language rule) for everything it
//! cannot prove renderable.
//!
//! In its own module rather than `expr.rs` (already past AGENTS.md's ~1,000
//! line decomposition threshold), following `narrow.rs`/`std_receiver.rs`/
//! `redeclaration.rs`.

use crate::env::Environment;
use crate::expr::infer_expr_in;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

/// The two surfaces that hand a value to `pycc_codegen`'s `to_str`
/// (`crates/pycc_codegen/src/lib.rs`, the f-string and `print` lowering).
///
/// The `str()` builtin is not a third surface. It is `C0001` via
/// `KNOWN_CALLABLE_BUILTINS` for every argument type **except**
/// [`Ty::Object`], which PR 4b of #1083 (Part 4 of #1026) admits as an
/// explicit conversion out of the opaque type. That admission needs nothing
/// from this module: `str(o)` infers to [`Ty::Str`], which reaches
/// [`reject_unrenderable`]'s catch-all arm and is renderable, and codegen's
/// `to_str` returns a `Scalar::Str` unchanged -- so the heap `PyStrObj` the
/// shim copies out of CPython is handled identically to a literal-derived
/// one. `print(o)` and `f"{o}"` on the object itself are admitted as well
/// since #1340 ([`reject_unrenderable`] has no [`Ty::Object`] arm): codegen
/// renders the object through the shim, `print` with `str()` and an
/// f-string with `format(o, '')`, exactly as CPython does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StringConversionSite {
    /// An argument of a `print(...)` call.
    PrintArgument,
    /// An `{expr}` interpolation inside an f-string.
    FStringInterpolation,
}

impl StringConversionSite {
    fn describe(self) -> &'static str {
        match self {
            StringConversionSite::PrintArgument => "a `print()` argument",
            StringConversionSite::FStringInterpolation => "an f-string interpolation",
        }
    }
}

/// Rejects a value that neither MIR rewrite can render as a string.
///
/// Every non-instance, non-protocol type passes through unchanged: this gate
/// covers only the instance/protocol shapes of #977. The tuple/container/
/// `Optional` `to_str` panics remain the documented follow-ups in
/// `docs/ROADMAP.md`.
///
/// The `Ty::Instance` arm's verdict, in the order it is decided:
///
/// 1. A builtin exception name (`pycc_hir::is_builtin_exception_class`)
///    is decided by *provenance*, never by the class's shape:
///    - seeded by HIR lowering (`env.is_synthetic_class`): accept.
///      `rewrite_exception_to_message` renders it -- the flat seven by name
///      (`pycc_mir::exception::resolve_exception_tag`), every other builtin
///      through its seeded class-table tag.
///    - absent from the class table (the shadow gate withheld seeding because
///      the module binds some *other* builtin exception name): only the flat
///      seven still render, by name, so they are accepted; every other
///      builtin gets no rewrite and would panic in codegen, so they are rejected.
///    - present but user-authored (plain or `@dataclass`) under any builtin
///      exception name: rejected. MIR resolves the name before the shape
///      (`rewrite_exception_to_message` runs first at both call sites in
///      `crates/pycc_mir/src/expr.rs`) and rewrites the plain instance to an
///      exception message, or applies a dataclass `__repr__` to an exception
///      object bound by `except*`, and the program crashes at runtime. This
///      deliberately also rejects a user `@dataclass` under a builtin exception
///      name past the flat seven that MIR would render today when instantiated
///      directly -- the one loss D-237 records.
/// 2. Any other name renders only through its synthesized dataclass
///    `__repr__`. A D-189 `exception_type_tag` on a user exception class is
///    *not* enough: MIR's class-table fallback would rewrite the plain
///    `PyInstanceObj` to `ExceptionMessage` and abort. A missing class-table
///    entry is reachable only through `pycc_types::check_function`'s empty
///    class table and is an ordinary rejection, not an internal error.
///
/// `Ty::Protocol` is rejected with its own message: a protocol-parameterised
/// body is checked once with the protocol type and its specialisations are
/// emitted by `monomorphize` without a second `infer_expr_in` pass, so the
/// checker cannot know the concrete class at the conversion site.
pub(crate) fn reject_unrenderable(
    env: &Environment,
    ty: &Ty,
    site: StringConversionSite,
) -> Result<(), Diagnostic> {
    match ty {
        Ty::Instance(name) => {
            let shadows_builtin_exception = pycc_hir::is_builtin_exception_class(name);
            let renderable = if shadows_builtin_exception {
                env.is_synthetic_class(name)
                    || (pycc_hir::is_flat_builtin_exception_class(name)
                        && env.lookup_class(name).is_none())
            } else {
                env.lookup_class(name).is_some_and(|def| def.is_dataclass)
            };
            if renderable {
                Ok(())
            } else {
                Err(unrenderable_instance(name, site, shadows_builtin_exception))
            }
        }
        Ty::Protocol(name) => Err(unrenderable_protocol(name, site)),
        // #1340: a CPython object renders, so it takes the catch-all.
        // Part 2 of #1026 refused it here because `pycc_codegen`'s `to_str`
        // had no object arm; codegen now converts an object operand before
        // `to_str` -- `print` through the shim's `PyObject_Str` helper in
        // its write phase, an f-string part through its `PyObject_Format`
        // helper (`crates/pycc_codegen/src/string_render.rs`) -- so no arm
        // is needed.
        // Part 1 of #1319: `pycc_codegen`'s `to_str` has no set arm, so
        // before this refusal `print({1})` passed `pycc check` and panicked
        // in the backend. `list`, `dict` and `tuple` are deliberately not
        // added here: #1220 owns rendering them.
        Ty::Set(_) | Ty::FrozenSet(_) => Err(crate::frozenset::unrenderable_set(ty)),
        // This catch-all also swallows every `Ty` variant added later: a new
        // variant `to_str` cannot render needs its own arm above.
        _ => Ok(()),
    }
}

/// Gates a string-conversion *expression*: [`reject_unrenderable`] on its
/// inferred type, then again on the value under every erased `cast`.
///
/// `pycc_mir` erases `cast(T, v)` to `v` and its `__repr__` rewrite keys on
/// the erased value's own class, so a representation-preserving upcast from
/// a non-dataclass subclass to a `@dataclass` base (`print(cast(Base, d))`
/// with `class Derived(Base): pass`) would pass a type-only gate as `Base`
/// and still reach codegen's `to_str` panic as a `Derived`. Only a call that
/// `infer_expr_in` itself routes to `check_cast` is looked through -- a user
/// `def cast` shadows the builtin there too -- and the descent repeats for a
/// nested `cast(A, cast(B, v))`. The value's own type is inferred exactly as
/// `check_cast` infers it, so no new diagnostic can surface from the descent.
pub(crate) fn reject_unrenderable_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
    site: StringConversionSite,
) -> Result<(), Diagnostic> {
    let ty = infer_expr_in(env, local_names, expr)?;
    reject_unrenderable(env, &ty, site)?;
    let mut current = expr;
    while let HirExpr::Call { callee, args } = current
        && callee == "cast"
        && env.lookup_function(callee).is_none()
    {
        // `infer_expr_in` above already ran `check_cast`, which rejects any
        // arity other than two and re-infers this very value before this
        // point, so the value's own inference cannot fail here.
        let value = &args[1];
        let value_ty = infer_expr_in(env, local_names, value)
            .expect("check_cast already inferred the cast value successfully");
        reject_unrenderable(env, &value_ty, site)?;
        current = value;
    }
    Ok(())
}

/// The `C0001` for a class instance neither MIR rewrite renders.
///
/// Reported at this crate's conventional `(0, 0)` span (rendered `1:1` by the
/// CLI): `pycc_types` carries no expression spans, and D-233's HIR-level
/// syntactic scan cannot apply because the value's *type* is unknown at HIR.
///
/// The help depends on why the instance is unrenderable. A class under any
/// other name becomes renderable by adding `@dataclass`. A user class
/// declared under one of the builtin exception names is rejected by name
/// before its shape is consulted, so `@dataclass` cannot help it (the
/// predicate never reaches the dataclass check for such a name); the only
/// remedy that works is renaming the class so it no longer shadows the
/// builtin.
fn unrenderable_instance(
    class_name: &str,
    site: StringConversionSite,
    shadows_builtin_exception: bool,
) -> Diagnostic {
    let help = if shadows_builtin_exception {
        format!(
            "print the instance's attributes individually, or rename `{class_name}` so it \
             no longer shadows the builtin exception `{class_name}`; adding `@dataclass` \
             does not make a class under a builtin exception name renderable"
        )
    } else {
        format!(
            "print the instance's attributes individually, or declare `{class_name}` with \
             `@dataclass` to get a synthesized `__repr__`"
        )
    };
    Diagnostic::error(
        "C0001",
        format!(
            "string conversion of a `{class_name}` instance as {} is not supported yet; \
             `print()` and f-string interpolation can render only a `@dataclass` instance \
             or a caught builtin exception",
            site.describe()
        ),
        Span::new(0, 0),
    )
    .with_help(help)
}

/// The `C0001` for a protocol-typed value; see [`reject_unrenderable`].
fn unrenderable_protocol(protocol_name: &str, site: StringConversionSite) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "string conversion of a value typed as protocol `{protocol_name}` as {} is not \
             supported yet; the concrete class is not known at the conversion site",
            site.describe()
        ),
        Span::new(0, 0),
    )
}

#[cfg(test)]
mod tests {
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
                 `@dataclass` to get a synthesized `__repr__`"
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

    /// Codex round 2 on PR #985: MIR erases `cast(Base, d)` to `d`, whose
    /// class is not a dataclass, so the gate must judge the erased value.
    #[test]
    fn a_cast_from_a_plain_subclass_to_a_dataclass_base_is_c0001_under_print() {
        assert_rejected(
            &format!("{UPCAST}print(cast(Base, d))\n"),
            "`Derived` instance",
        );
    }

    #[test]
    fn a_cast_from_a_plain_subclass_to_a_dataclass_base_is_c0001_in_an_fstring() {
        assert_rejected(
            &format!("{UPCAST}s: str = f\"{{cast(Base, d)}}\"\n"),
            "`Derived` instance as an f-string interpolation",
        );
    }

    #[test]
    fn a_nested_cast_is_looked_through_to_the_erased_value() {
        assert_rejected(
            &format!("{UPCAST}print(cast(Base, cast(Derived, d)))\n"),
            "`Derived` instance",
        );
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
    fn a_user_written_repr_does_not_make_the_instance_renderable() {
        // MIR never calls a non-dataclass `__repr__`; honouring it is the
        // deferred positive slice, so this test flips visibly when it lands.
        assert_rejected(
            "class C:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n\
             \x20   def __repr__(self) -> str:\n        return \"C()\"\n\n\
             print(C(1))\n",
            "`C` instance",
        );
    }

    #[test]
    fn an_enum_member_is_c0001() {
        assert_rejected(
            "from enum import Enum\n\nclass Color(Enum):\n    RED = 1\n    BLUE = 2\n\n\
             print(Color.RED)\n",
            "`Color` instance",
        );
    }

    #[test]
    fn a_non_dataclass_subclass_of_a_dataclass_is_c0001() {
        assert_rejected(
            &format!("{DATACLASS}class Q(P):\n    pass\n\nprint(Q(1, 2))\n"),
            "`Q` instance",
        );
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
}
