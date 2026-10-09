//! The `__init__`-body attribute-slot pre-scan, extracted from `super`
//! (`class.rs`) per AGENTS.md's decomposition rule when #1262 widened it.
//!
//! [`collect_init_attrs`] walks `__init__`'s own top-level statements for
//! `<receiver>.<attr> = <value>` assignments, and `slot_ty_from_init_rhs`
//! derives each attribute slot's `Ty` structurally from the first
//! assignment's right-hand side. #1262 (Part 1 of #1218) admits a
//! `list[int]`/`dict[str, int]` parameter there, stored as a pointer word
//! (D-154's slot layout, leak-only per D-107/D-124). #1264 (Part 3) adds
//! the annotated `<receiver>.<attr>: list[int] = []` / `dict[str, int] = {}`
//! form, whose slot `Ty` is its written annotation's. #1265 (Part 4) admits
//! the unannotated `<receiver>.<attr> = []` as a *provisional*
//! `list[<Ty::Infer>]` slot: `pycc_types`' empty-container pass resolves its
//! element type class-wide, and its gate refuses any provisional slot it
//! could not resolve (D-245's 2026-09-24 amendment for #1265). The
//! unannotated `{}` stays `C0001` until its producer exists (#891).
//!
//! #1266 (Part 5) lets a class-body declaration (`n: int`,
//! `d: dict[str, int]`, see `super::declared_attrs`) supply the slot type:
//! the establishing assignment then records the **declared** type, the RHS
//! passing only `slot_ty_from_init_rhs`'s shape gate (`declared_slot_ty`).
//! That is also what admits an establishing `{}` -- under a `dict[K, V]`
//! declaration the slot is concrete, and `pycc_types`' existing reset
//! rewrite types the literal.
//!
//! Part 1 of #1367 admits a parameter annotated with a class a foreign
//! import binds (`Ty::Object`), declared or not.
//!
//! #1388 lifts the shape gate for a declared attribute: its establishing
//! right-hand side may be any expression the body lowers -- a call, a
//! subscript, arithmetic, a read of a slot assigned earlier -- because the
//! declaration, not the expression, supplies the slot type, and
//! `pycc_types::check_attr_set` judges the value against it. Reading a slot
//! before its assignment is no longer a compile-time concern either: the
//! runtime raises CPython's `AttributeError` for it
//! (`pycc_rt::instance`). The gate stays in force where type parameters are
//! involved (see `declared_slot_ty`) and for an undeclared attribute, whose
//! slot type has no other source.

use super::declared_attrs::DeclaredAttr;
use crate::{Ty, unsupported};
use pycc_ast::{Expr, Number, Stmt};
use pycc_diag::Diagnostic;

/// Scans `__init__`'s own top-level body statements (no recursion into a
/// nested `if`/`while`/`for` -- see `super`'s own module doc comment) for
/// `self.<attr> = <value>` assignments and annotated `self.<attr>: <T> =
/// <value>` ones (#1264), building the attribute-slot list in
/// first-assignment source order across both statement kinds. An
/// unannotated `self.<attr> = []` records the provisional
/// `list[<Ty::Infer>]` slot (#1265; see `slot_ty_from_init_rhs`). Only the *first* assignment to a given
/// attribute name establishes its slot and `Ty`; a later `self.<attr> =
/// ...` reassignment further down `__init__`'s own body is structurally
/// ignored here (it is still lowered normally by `stmt::lower_body` into an
/// ordinary `HirStmt::AttrSet`, and `pycc_types` checks its value against
/// the already-established slot type -- this pre-scan's only job is
/// deciding *which* attributes exist and their *first-assignment* type).
///
/// `params` is `lower_method`'s own full parameter list (whose first entry
/// is always the *canonical* receiver name, `self`, whatever the source
/// spelled -- see `super::receiver`) -- used to resolve a
/// bare-parameter-name RHS's `Ty`.
///
/// `declared` carries the class body's instance attribute declarations
/// (#1266). For a declared attribute, the first plain assignment records the
/// declared type (see `declared_slot_ty`), and a first annotated assignment
/// must name that same type.
///
/// `annotation_ty` resolves an annotated assignment's annotation to its
/// `Ty`. The caller passes the same `annotation_to_ty` call, in the same
/// context, that `stmt::ann_assign` used when it lowered and accepted this
/// body, so the slot type is exactly the type of the value it built.
///
/// #1181: `receiver_name` is the receiver's *source* spelling, which is what
/// the body actually writes. Comparing against the literal `self` instead
/// would make `def __init__(this): this.v = 1` establish zero attribute
/// slots, and every later read of `c.v` would fail with `T0044`.
pub(super) fn collect_init_attrs(
    init_body: &[Stmt],
    params: &[(String, Ty)],
    receiver_name: &str,
    declared: &DeclaredAttrs<'_>,
    annotation_ty: &dyn Fn(&Expr) -> Result<Ty, Diagnostic>,
) -> Result<Vec<(String, Ty)>, Diagnostic> {
    let mut attrs: Vec<(String, Ty)> = Vec::new();
    for stmt in init_body {
        // #1264 (Part 3 of #1218): `self.xs: list[int] = []` declares the
        // slot with the annotation's type. `stmt::lower_body` has already
        // lowered this body, and its `ann_assign` arm admits an attribute
        // target only as a `list[int]`/`dict[str, int]` annotation with the
        // matching empty literal, so `annotation_ty` -- the same
        // `annotation_to_ty` call, in the same context -- resolves it again
        // to exactly the type that arm built the value from.
        if let Stmt::AnnAssign(ann) = stmt {
            if let Some(attr_name) = receiver_attr(&ann.target, receiver_name)
                && !attrs.iter().any(|(name, _)| *name == attr_name)
            {
                let ty = annotation_ty(&ann.annotation)?;
                // #1266: a class-body declaration and the establishing
                // annotated assignment must name the same type.
                if let Some(decl) = declared.find(&attr_name)
                    && decl.ty != ty
                {
                    return Err(unsupported(
                        format!(
                            "instance attribute `{attr_name}` is declared in class `{}` as \
                             `{}` but annotated `{}` in `__init__` -- the two annotations must \
                             agree",
                            declared.class_name,
                            decl.ty.name(),
                            ty.name()
                        ),
                        ann.range,
                    ));
                }
                attrs.push((attr_name, ty));
            }
            continue;
        }
        let Stmt::Assign(assign) = stmt else {
            continue;
        };
        // #1213: a chained assignment (`self.x = self.y = 0`) declares
        // every receiver attribute among its targets, each typed from the
        // one shared right-hand side, exactly as the single-target
        // assignments `stmt::lower_stmt_expanded` expands it into would.
        for target in &assign.targets {
            let Some(attr_name) = receiver_attr(target, receiver_name) else {
                continue;
            };
            if attrs.iter().any(|(name, _)| *name == attr_name) {
                continue;
            }
            let ty = match declared.find(&attr_name) {
                Some(decl) => declared_slot_ty(decl, declared, assign, params, receiver_name)?,
                None => slot_ty_from_init_rhs(
                    &assign.value,
                    params,
                    receiver_name,
                    (!declared.generic).then_some(attr_name.as_str()),
                )?,
            };
            attrs.push((attr_name, ty));
        }
    }
    Ok(attrs)
}

/// A class body's instance attribute declarations (#1266), as the `__init__`
/// pre-scan consumes them: the list [`super::declared_attrs`] built, plus
/// the class name its diagnostics quote.
pub(super) struct DeclaredAttrs<'a> {
    /// The declarations, in source order.
    pub(super) attrs: &'a [DeclaredAttr],
    /// The class's own name.
    pub(super) class_name: &'a str,
    /// Whether the class is a PEP 695 generic class (`class C[T]:`), whose
    /// declarations keep the shape gate (#1388; see `declared_slot_ty`).
    pub(super) generic: bool,
}

impl DeclaredAttrs<'_> {
    /// The declaration of `name`, if the class body declares it.
    fn find(&self, name: &str) -> Option<&DeclaredAttr> {
        self.attrs.iter().find(|decl| decl.name == name)
    }
}

/// The slot type of a declared attribute's establishing plain assignment
/// (#1266): the **declared** type, never the right-hand side's.
///
/// An empty `[]`/`{}` takes the declared type without consulting
/// [`slot_ty_from_init_rhs`]: under a matching `list[int]`/`dict[str, int]`
/// declaration `pycc_types::empty_container`'s `rewrite_attr_resets` types
/// the literal from the now-concrete slot (this is how the unannotated `{}`
/// becomes admissible), and under any other declaration the literal stays
/// untyped and the checker reports the same wrong-shape `T0003` a later
/// reset gets.
///
/// Since #1388 any other right-hand side is admitted too, its type
/// discarded: `pycc_types::check_attr_set` judges the value against the
/// declared slot as for every `AttrSet`, and a read of a slot not yet
/// assigned raises `AttributeError` at run time. The exception is the one
/// pair that check cannot judge -- a type parameter against anything else,
/// since `is_assignable` admits any scalar into a `Ty::Param` slot and a
/// `Ty::Param` into any scalar slot. In a PEP 695 generic class, the only
/// place a type parameter can occur, the right-hand side must still pass
/// `slot_ty_from_init_rhs`'s shape gate, whose bare-parameter answer is
/// exact, and a mismatched pair is refused here, so it cannot compile into
/// a slot that holds the wrong representation for some instantiation.
fn declared_slot_ty(
    decl: &DeclaredAttr,
    declared: &DeclaredAttrs<'_>,
    assign: &pycc_ast::StmtAssign,
    params: &[(String, Ty)],
    receiver_name: &str,
) -> Result<Ty, Diagnostic> {
    let class_name = declared.class_name;
    let value = assign.value.as_ref();
    let is_empty_display = matches!(value, Expr::List(list) if list.elts.is_empty())
        || matches!(value, Expr::Dict(dict) if dict.items.is_empty());
    if is_empty_display {
        return Ok(decl.ty.clone());
    }
    // A type parameter is in scope only inside a PEP 695 generic class: a
    // generic method is refused (`C0001`) and a `Generic[T]` base is erased
    // (`super::generic_base`), so neither a declaration nor an `__init__`
    // parameter of any other class can carry a `Ty::Param`.
    if !declared.generic {
        return Ok(decl.ty.clone());
    }
    let inferred = slot_ty_from_init_rhs(value, params, receiver_name, None)?;
    let involves_param = matches!(inferred, Ty::Param(_)) || matches!(decl.ty, Ty::Param(_));
    if involves_param && inferred != decl.ty {
        return Err(unsupported(
            format!(
                "instance attribute `{}` declared in class `{class_name}` as `{}` is assigned a \
                 value of type `{}` -- a type-parameter attribute must be assigned a value of \
                 that same type parameter, and a concrete attribute cannot hold one",
                decl.name,
                decl.ty.name(),
                inferred.name()
            ),
            assign.range,
        ));
    }
    Ok(decl.ty.clone())
}

/// The attribute name `target` assigns when it is `<receiver>.<attr>` on
/// the receiver's own source spelling (#1181), and `None` for any other
/// target shape -- a bare name, a subscript, or an attribute of some other
/// base (including a nested `self.x.y`).
fn receiver_attr(target: &Expr, receiver_name: &str) -> Option<String> {
    let Expr::Attribute(attr) = target else {
        return None;
    };
    let Expr::Name(receiver) = attr.value.as_ref() else {
        return None;
    };
    (receiver.id.as_str() == receiver_name).then(|| attr.attr.to_string())
}

/// Resolves an instance attribute's slot `Ty` from its first-assignment RHS
/// inside `__init__`, structurally -- see `super`'s own module doc comment
/// and `lower_method`'s doc comment for why this must not require a real
/// type-inference pass: only a bare reference to one of `__init__`'s own
/// (always-annotated) parameters, a scalar literal, or -- since #1265 -- an
/// empty list display is accepted. A parameter may be a scalar
/// (int/float/bool/str), a PEP 695 type parameter, or -- since #1262 -- a
/// `list[int]`/`dict[str, int]` container, which the slot stores as its
/// pointer word, or -- since Part 1 of #1367 -- a class a foreign import
/// binds (`Ty::Object`), stored as its `PyObject*` word, or -- since #1389
/// -- a class of this program (`Ty::Instance`), stored as its instance
/// pointer word. For an attribute a
/// class-body declaration types (#1266), this is only the shape gate, and
/// since #1388 only where a type parameter is in scope: `declared_slot_ty`
/// discards the type it returns in favor of the declared one, and admits
/// any right-hand side everywhere else.
///
/// `[]` yields the provisional slot type `list[<Ty::Infer>]`. It is the one
/// place this crate records a `Ty::Infer` slot on purpose, and it never
/// reaches `pycc_mir`: `pycc_types::empty_container` replaces it with the
/// element type of an inherited declaration or of the first
/// `self.<attr>.append(v)` in the class's own methods, and refuses the
/// program with `T0003` when neither exists. An empty dict display has no
/// such producer yet (`self.d[k] = v` is #891), so it gets its own `C0001`
/// naming the two spellings that work: the annotated assignment and a
/// class-body declaration, whose type `declared_slot_ty` supplies without
/// calling this function for an empty literal. Every other RHS shape --
/// including an arithmetic expression, a call, or a reference to `self`
/// itself -- is `C0001`. `declaration_hint` is the attribute's name when a
/// class-body declaration would admit that shape (#1388: an undeclared
/// attribute outside a PEP 695 generic class), and the diagnostic then
/// names the declaration; it is `None` where a declaration would not help.
fn slot_ty_from_init_rhs(
    value: &Expr,
    params: &[(String, Ty)],
    receiver_name: &str,
    declaration_hint: Option<&str>,
) -> Result<Ty, Diagnostic> {
    match value {
        // Two guarded arms of the same top-level `match`, deliberately
        // *asymmetric* rather than two structurally identical `matches!`
        // checks (`Number::Int(_)` / `Number::Float(_)`): every symmetric
        // shape tried for this Int/Float split -- two standalone `if let
        // ... && matches!(..)` chains, a nested `match &lit.value { .. }`
        // behind one outer `if let`, an `Option`-valued intermediate
        // `match`, a single outer `if let` wrapping two independent bare
        // `if matches!(..)` checks, and even two guarded arms of this same
        // trailing `match` when both guards called `matches!` against a
        // distinct `Number` variant -- reported the *second* of the two as
        // an uncovered region under `cargo llvm-cov`, regardless of which
        // variant it checked or what control-flow shape wrapped it, even
        // though it demonstrably executes (both
        // `an_init_attr_assigned_an_int_literal_establishes_an_int_slot`
        // and `an_init_attr_assigned_a_float_literal_establishes_a_float_slot`
        // below pass, each asserting the exact `Ty` this arm resolves to).
        // The common factor was always two source-adjacent regions with
        // byte-for-byte identical `matches!(lit.value, Number::<Variant>(_))`
        // shapes differing only in the variant name -- consistent with an
        // LLVM coverage-mapping counter getting deduplicated/shared across
        // two structurally-identical-looking regions, so only the first is
        // ever marked hit. Writing the second guard as a negation of the
        // first (`!matches!(.., Number::Int(_))`, rather than its own
        // positive `matches!(.., Number::Float(_))`) breaks that structural
        // symmetry and resolves it -- confirmed clean at 100% region
        // coverage for this file with this exact shape, after every
        // symmetric alternative above reproduced the identical artifact.
        Expr::NumberLiteral(lit) if matches!(lit.value, Number::Int(_)) => Ok(Ty::Int),
        Expr::NumberLiteral(lit) if !matches!(lit.value, Number::Int(_)) => Ok(Ty::Float),
        // The second arm's negation also correctly subsumes
        // `Number::Complex` (`1j`), which can never actually reach this
        // function: `expr::lower_expr`'s own `NumberLiteral` arm has no
        // case for `Number::Complex`, so `stmt::lower_body` (called before
        // this pre-scan ever runs, see `lower_class`) always rejects
        // `self.x = 1j` with `C0001` first, confirmed directly by running
        // this exact snippet through `lower_checked` rather than assumed.
        // A provably-unreachable `Number::Complex` value being classified
        // as `Ty::Float` by the negation above is therefore never
        // observable from any real parsed source.
        Expr::Name(name) => {
            // #1181: `params[0].0` is the *canonical* receiver name, so an
            // RHS naming the receiver has to be resolved through the
            // source spelling -- otherwise `def __init__(this, v: int):
            // this.x = this` takes the unresolvable-name path below instead
            // of its `self`-spelled twin's `Ty::Instance` path.
            let lookup = if name.id.as_str() == receiver_name {
                super::receiver::CANONICAL_RECEIVER
            } else {
                name.id.as_str()
            };
            let resolved = params
                .iter()
                .find(|(param_name, _)| param_name == lookup)
                .map(|(_, ty)| ty.clone());
            match resolved {
                // A scalar-typed parameter (int/float/bool/str) seeds a
                // slot holding the value itself in `pycc_rt::instance`'s
                // single `i64` word per slot (D-154's own class-instance-
                // layout ADR). #1262: a `list[int]`/`dict[str, int]`
                // parameter seeds a slot holding the container's pointer,
                // reinterpreted as that word exactly like a `str` pointer
                // is; containers stay leak-only (D-107, D-124), so the
                // slot store needs no refcount traffic and a read returns
                // the same object (CPython's aliasing). `check_container_ty`
                // has already validated the parameter's annotation, so any
                // `Ty::List`/`Ty::Dict` reaching here is one of those two
                // admitted shapes. Part 1 of #1367: a `Ty::Object`
                // parameter -- one annotated with a class a foreign import
                // binds -- seeds a slot holding its `PyObject*`, the same
                // pointer word; every `object` producer hands back a
                // reference that is never released, so the store needs no
                // refcount traffic either. #1389: a `Ty::Instance`
                // parameter -- an instance of a class of this program,
                // including the receiver itself (`self.link = self`) --
                // seeds a slot holding its instance pointer; `pycc_rt`
                // never frees an instance, so the store needs no refcount
                // traffic and a read aliases the stored instance, as in
                // CPython. Every other non-scalar type -- a `set[T]`, a
                // by-value `tuple[...]`, an `Optional`, or a protocol --
                // has an ownership or carrier question of its own and stays
                // refused.
                //
                // PEP 695 (#387): `Ty::Param` is also accepted — a generic
                // class's `__init__` parameter typed `T` seeds a slot with
                // `Ty::Param("T")`, which is substituted with a concrete
                // scalar type at monomorphization time (reusing PR-13's
                // D-133/D-134 call-site-substitution mechanism). At runtime
                // the slot is still a single `i64` word, so the type
                // parameter is purely compile-time.
                Some(
                    ty @ (Ty::Int
                    | Ty::Float
                    | Ty::Bool
                    | Ty::Str
                    | Ty::Param(_)
                    | Ty::List(_)
                    | Ty::Dict(..)
                    | Ty::Object
                    | Ty::Instance(_)),
                ) => Ok(ty),
                Some(other) => Err(unsupported(
                    format!(
                        "`{receiver_name}.<attr> = {}` cannot establish an attribute of type \
                         `{}` yet -- only a scalar (int/float/bool/str), `list[int]`, \
                         `dict[str, int]`, class instance or foreign-imported class parameter \
                         is supported",
                        name.id,
                        other.name()
                    ),
                    pycc_ast::expr_range(value),
                )),
                None => Err(unsupported(
                    format!(
                        "`{receiver_name}.<attr> = {}` must reference one of `__init__`'s own \
                         parameters to establish the attribute's type, or use a scalar \
                         literal",
                        name.id
                    ),
                    pycc_ast::expr_range(value),
                )),
            }
        }
        Expr::BooleanLiteral(_) => Ok(Ty::Bool),
        Expr::StringLiteral(_) => Ok(Ty::Str),
        // #1265 (Part 4 of #1218): the provisional slot -- see this
        // function's doc comment.
        Expr::List(list) if list.elts.is_empty() => Ok(Ty::List(Box::new(Ty::Infer))),
        Expr::Dict(dict) if dict.items.is_empty() => Err(unsupported(
            format!(
                "an unannotated `{receiver_name}.<attr> = {{}}` has no key/value type source \
                 yet -- annotate it (`{receiver_name}.d: dict[str, int] = {{}}`) or declare it \
                 in the class body (`d: dict[str, int]`); inferring it from \
                 `{receiver_name}.d[k] = v` needs a subscript store on an attribute receiver \
                 (#891)"
            ),
            pycc_ast::expr_range(value),
        )),
        other => {
            let hint = declaration_hint.map_or(String::new(), |attr| {
                format!(
                    " -- or declare it in the class body (`{attr}: <type>`), which admits any \
                     expression"
                )
            });
            Err(unsupported(
                format!(
                    "an instance attribute's first assignment inside `__init__` must be a bare \
                     parameter name, a scalar literal (int/float/bool/str) or an empty list \
                     `[]` whose element type the class's own methods supply, so its type is \
                     known at compile time{hint}"
                ),
                pycc_ast::expr_range(other),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Ty;
    use crate::class::tests::{assert_c0001, lower_ok};

    #[test]
    fn an_init_attr_assigned_from_an_unrelated_name_is_unsupported() {
        assert_c0001("class C:\n    def __init__(self, x: int) -> None:\n        self.y = z\n");
    }

    #[test]
    fn an_init_attr_assigned_from_self_establishes_an_instance_slot() {
        // #1389: the receiver is a `Ty::Instance` parameter like any other.
        let hir = lower_ok("class C:\n    def __init__(self) -> None:\n        self.link = self\n");
        assert_eq!(
            hir.class_defs[0].1.attrs,
            vec![("link".to_string(), Ty::Instance(Box::new("C".to_string())))]
        );
    }

    #[test]
    fn an_init_attr_assigned_from_an_instance_param_establishes_an_instance_slot() {
        // #1389: an instance of another class of this program.
        let hir = lower_ok(
            "class A:\n    def __init__(self) -> None:\n        self.n = 1\n\n\n\
             class B:\n    def __init__(self, a: A) -> None:\n        self.a = a\n",
        );
        assert_eq!(
            hir.class_defs[1].1.attrs,
            vec![("a".to_string(), Ty::Instance(Box::new("A".to_string())))]
        );
    }

    fn c0001_message(source: &str) -> String {
        let module = crate::pycc_parser_test_helper::parse(source);
        let diagnostic = crate::lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001", "source: {source:?}");
        diagnostic.message
    }

    #[test]
    fn an_init_attr_assigned_from_a_list_param_establishes_a_list_slot() {
        // #1262: the slot holds the list's pointer word.
        let hir = lower_ok(
            "class C:\n    def __init__(self, xs: list[int]) -> None:\n        self.xs = xs\n",
        );
        assert_eq!(
            hir.class_defs[0].1.attrs,
            vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))]
        );
    }

    #[test]
    fn an_init_attr_assigned_from_a_dict_param_establishes_a_dict_slot() {
        let hir = lower_ok(
            "class C:\n    def __init__(self, d: dict[str, int]) -> None:\n        self.d = d\n",
        );
        assert_eq!(
            hir.class_defs[0].1.attrs,
            vec![("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))))]
        );
    }

    #[test]
    fn an_init_attr_assigned_from_a_set_or_tuple_param_names_the_accepted_set() {
        for annotation in ["set[int]", "tuple[int, int]"] {
            let message = c0001_message(&format!(
                "class C:\n    def __init__(self, s: {annotation}) -> None:\n        self.s = s\n"
            ));
            assert!(
                message.contains(&format!(
                    "`self.<attr> = s` cannot establish an attribute of type `{annotation}` yet"
                )),
                "{message}"
            );
            assert!(
                message.contains(
                    "only a scalar (int/float/bool/str), `list[int]`, `dict[str, int]`, class \
                     instance or foreign-imported class parameter is supported"
                ),
                "{message}"
            );
        }
    }

    #[test]
    fn an_init_attr_assigned_an_empty_list_literal_establishes_a_provisional_slot() {
        // #1265 (Part 4 of #1218): the element type is left for
        // `pycc_types::empty_container` to resolve class-wide.
        let hir = lower_ok("class C:\n    def __init__(self) -> None:\n        self.xs = []\n");
        assert_eq!(
            hir.class_defs[0].1.attrs,
            vec![("xs".to_string(), Ty::List(Box::new(Ty::Infer)))]
        );
    }

    #[test]
    fn a_this_spelled_empty_list_establishes_a_provisional_slot() {
        // #1181: the receiver is whatever the first parameter is called.
        let hir = lower_ok("class C:\n    def __init__(this) -> None:\n        this.xs = []\n");
        assert_eq!(
            hir.class_defs[0].1.attrs,
            vec![("xs".to_string(), Ty::List(Box::new(Ty::Infer)))]
        );
    }

    #[test]
    fn an_init_attr_assigned_an_empty_dict_literal_names_the_annotated_spelling() {
        // The dict half has no producer yet: `self.d[k] = v` is #891. The
        // message names both spellings that do supply a type: the annotated
        // assignment (#1264) and a class-body declaration (#1266).
        let message =
            c0001_message("class C:\n    def __init__(this) -> None:\n        this.d = {}\n");
        assert_eq!(
            message,
            "an unannotated `this.<attr> = {}` has no key/value type source yet -- annotate \
             it (`this.d: dict[str, int] = {}`) or declare it in the class body (`d: dict[str, \
             int]`); inferring it from `this.d[k] = v` needs a subscript store on an attribute \
             receiver (#891)"
        );
    }

    #[test]
    fn a_chained_empty_list_initialiser_stays_refused() {
        // CPython binds one list to both attributes; two separate `[]`
        // evaluations would break that identity, so the chain refusal (which
        // runs before this pre-scan) is load-bearing.
        let message = c0001_message(
            "class C:\n    def __init__(self) -> None:\n        self.a = self.b = []\n",
        );
        assert!(
            message.starts_with("chained assignment of an empty `[]`/`{}` literal"),
            "{message}"
        );
    }

    #[test]
    fn any_other_init_rhs_names_every_admitted_shape() {
        let message =
            c0001_message("class C:\n    def __init__(self) -> None:\n        self.xs = [1]\n");
        assert!(
            message.contains(
                "must be a bare parameter name, a scalar literal (int/float/bool/str) or an \
                 empty list `[]`"
            ),
            "{message}"
        );
    }

    #[test]
    fn an_init_attr_assigned_an_int_literal_establishes_an_int_slot() {
        let hir = lower_ok("class C:\n    def __init__(self) -> None:\n        self.x = 5\n");
        assert_eq!(hir.class_defs[0].1.attrs, vec![("x".to_string(), Ty::Int)]);
    }

    #[test]
    fn an_init_attr_assigned_a_float_literal_establishes_a_float_slot() {
        let hir = lower_ok("class C:\n    def __init__(self) -> None:\n        self.x = 1.5\n");
        assert_eq!(
            hir.class_defs[0].1.attrs,
            vec![("x".to_string(), Ty::Float)]
        );
    }

    #[test]
    fn an_init_attr_assigned_a_complex_literal_is_unsupported() {
        // `1j` fails to lower long before `collect_init_attrs`'s own
        // pre-scan ever runs -- see `slot_ty_from_init_rhs`'s own comment
        // on its guarded `NumberLiteral` arms for why.
        assert_c0001("class C:\n    def __init__(self) -> None:\n        self.x = 1j\n");
    }

    #[test]
    fn an_init_attr_assigned_a_bool_literal_establishes_a_bool_slot() {
        let hir = lower_ok("class C:\n    def __init__(self) -> None:\n        self.x = True\n");
        assert_eq!(hir.class_defs[0].1.attrs, vec![("x".to_string(), Ty::Bool)]);
    }

    #[test]
    fn an_init_attr_assigned_a_string_literal_establishes_a_str_slot() {
        let hir = lower_ok("class C:\n    def __init__(self) -> None:\n        self.x = \"hi\"\n");
        assert_eq!(hir.class_defs[0].1.attrs, vec![("x".to_string(), Ty::Str)]);
    }

    #[test]
    fn an_init_attr_assigned_an_arithmetic_expression_is_unsupported() {
        assert_c0001("class C:\n    def __init__(self, x: int) -> None:\n        self.y = x + 1\n");
    }

    #[test]
    fn a_second_assignment_to_the_same_init_attr_does_not_change_its_slot_type() {
        // The pre-scan only records the *first* assignment to a given
        // attribute name; a later `self.x = ...` inside `__init__` itself
        // is still lowered normally (as a second `HirStmt::AttrSet`), but
        // does not add a second slot or change the recorded type.
        let hir = lower_ok(
            "class C:\n    def __init__(self, x: int) -> None:\n        self.x = x\n        self.x = 0\n",
        );
        assert_eq!(hir.class_defs[0].1.attrs, vec![("x".to_string(), Ty::Int)]);
    }

    #[test]
    fn non_attribute_statements_inside_init_are_ignored_by_the_pre_scan() {
        // Exercises every early-`continue` guard in `collect_init_attrs`
        // that a non-`self.<attr> = <value>` statement can reach without
        // itself being rejected by the rest of the pipeline: a plain local
        // assignment (not an `Expr::Attribute` target) and an attribute
        // assignment on a receiver other than `self` are both simply
        // skipped by the pre-scan -- none of them contributes an attribute
        // slot, and none of them is rejected by this pass (later lowering
        // of the method body may still reject some of them for other
        // reasons; this pre-scan's own job is only to skip them, not judge
        // them).
        let hir = lower_ok(
            "class C:\n    def __init__(self, x: int) -> None:\n        y = 1\n        other.z = 1\n        self.x = x\n",
        );
        assert_eq!(hir.class_defs[0].1.attrs, vec![("x".to_string(), Ty::Int)]);
    }

    #[test]
    fn a_chained_assignment_inside_init_declares_every_receiver_attribute_it_targets() {
        // #1213: `self.x = self.y = 0` declares both `x` and `y`, each typed
        // from the one shared right-hand side; a non-receiver target in the
        // same chain (`n`) and a repeat of an already-declared attribute add
        // no slot.
        let hir = lower_ok(
            "class C:\n    def __init__(self, v: int) -> None:\n        self.x = self.y = 0\n        n = self.y = self.z = v\n",
        );
        assert_eq!(
            hir.class_defs[0].1.attrs,
            vec![
                ("x".to_string(), Ty::Int),
                ("y".to_string(), Ty::Int),
                ("z".to_string(), Ty::Int),
            ]
        );
    }

    #[test]
    fn an_attribute_assignment_on_a_nested_attribute_base_inside_init_is_ignored_by_the_pre_scan() {
        // `self.x.y = 0` -- the outer `Attribute`'s own `.value` is itself
        // an `Attribute` (`self.x`), not a bare `Expr::Name`, so the `let
        // Expr::Name(receiver) = attr.value.as_ref() else { continue }`
        // guard's own early-exit fires and this statement contributes no
        // attribute slot. Structurally this still lowers successfully at
        // the HIR level (attribute access/assignment is generic over any
        // base expression, D-154's own `HirExpr::AttrGet`/`HirStmt::AttrSet`
        // doc comments) -- `pycc_types` is what would reject `self.x.y = 0`
        // once `x` turns out not to be a declared attribute of any
        // instance type, which is out of this crate's own scope to assert
        // on here.
        let hir = lower_ok("class C:\n    def __init__(self) -> None:\n        self.x.y = 0\n");
        assert_eq!(hir.class_defs[0].1.attrs, Vec::<(String, Ty)>::new());
    }

    #[test]
    fn an_annotated_empty_list_in_init_establishes_a_list_slot() {
        // #1264: the slot type is the written annotation's.
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        self.xs: list[int] = []\n",
        );
        assert_eq!(
            hir.class_defs[0].1.attrs,
            vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))]
        );
    }

    #[test]
    fn an_annotated_empty_dict_in_init_establishes_a_dict_slot() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        self.d: dict[str, int] = {}\n",
        );
        assert_eq!(
            hir.class_defs[0].1.attrs,
            vec![("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))))]
        );
    }

    #[test]
    fn an_annotated_slot_follows_the_receivers_source_spelling() {
        // #1181: the receiver is whatever the first parameter is called.
        let hir = lower_ok(
            "class C:\n    def __init__(this) -> None:\n        this.xs: list[int] = []\n",
        );
        assert_eq!(
            hir.class_defs[0].1.attrs,
            vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))]
        );
    }

    #[test]
    fn annotated_and_plain_init_assignments_share_first_assignment_wins_ordering() {
        // A later plain or annotated store to an already-declared attribute
        // adds no slot; slots keep source order across both statement kinds;
        // a non-receiver annotated target and an annotated local add none.
        let hir = lower_ok(
            "class C:\n    def __init__(self, n: int, xs: list[int]) -> None:\n        \
             self.n = n\n        self.xs: list[int] = []\n        self.xs = xs\n        \
             self.xs: list[int] = []\n        k: int = 1\n        \
             other.ys: list[int] = []\n        self.d: dict[str, int] = {}\n",
        );
        assert_eq!(
            hir.class_defs[0].1.attrs,
            vec![
                ("n".to_string(), Ty::Int),
                ("xs".to_string(), Ty::List(Box::new(Ty::Int))),
                ("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int)))),
            ]
        );
    }

    #[test]
    fn an_annotated_store_nested_in_an_init_block_declares_no_slot() {
        // Only `__init__`'s top-level statements declare a slot, exactly as
        // for a plain `self.x = ...`; the store itself still lowers and
        // `pycc_types` reports the missing attribute (`T0044`).
        let hir = lower_ok(
            "class C:\n    def __init__(self, c: bool) -> None:\n        if c:\n            \
             self.xs: list[int] = []\n",
        );
        assert_eq!(hir.class_defs[0].1.attrs, Vec::<(String, Ty)>::new());
    }
}
