//! Comprehension checking (PR-12, D-117; #1254, D-250): the iterable's
//! element type, the produced container type and its element gate, the
//! `name = <comp>` statement forms at module and function scope, and the
//! expression form, whose loop variable is bound only in a scoped clone of
//! the environment.
//!
//! Extracted from `lib.rs` per AGENTS.md's file-decomposition rule (D-185
//! tracking issue #544). The statement arms in `check_stmt` (module scope,
//! `local_names` empty) and `check_stmt_in_function` each build a
//! [`CompView`] and call [`check_comp_assign`], so both scopes share one
//! body that differs only in `local_names` -- `infer_expr(env, e)` is exactly
//! `infer_expr_in(env, &[], e)`.

use crate::{
    Environment, check_assignment, check_range_operand_in, infer_expr_in, lookup_bound_name,
};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{CompElt, CompIter, HirComprehension, HirExpr, Ty};

/// The element expressions of one comprehension, by kind.
pub(crate) enum CompElts<'a> {
    /// `[elt for ...]`.
    List(&'a HirExpr),
    /// `{elt for ...}`.
    Set(&'a HirExpr),
    /// `{key: value for ...}`.
    Dict(&'a HirExpr, &'a HirExpr),
}

/// A borrowed view of one comprehension's parts.
pub(crate) struct CompView<'a> {
    /// The D-117 synthesized loop-variable name.
    pub(crate) var: &'a str,
    pub(crate) iter: &'a CompIter,
    pub(crate) cond: Option<&'a HirExpr>,
    pub(crate) elts: CompElts<'a>,
}

/// Resolves a comprehension's iterable (`pycc_hir::CompIter`) to the loop
/// variable's type, without binding it -- mirrors `HirStmt::ForList`'s own
/// resolution exactly (`check_stmt`/`check_stmt_in_function`'s existing
/// `ForList` arms), reused rather than duplicated a third time (PR-12,
/// D-117). Range/list/dict/set element-type resolution is identical to
/// `ForList`'s; a comprehension adds nothing new here. Since #1344 that
/// includes a `set[C]`/`frozenset[C]` of user-class instances, whose loop
/// variable is the instance.
///
/// An iterable expression ([`CompIter::Iterable`], Part 1 of #1255) must be
/// a CPython object (D-258), whose loop variable is an object too; every
/// other type is refused with [`non_object_iterable`].
pub(crate) fn resolve_comp_iter(
    env: &Environment,
    local_names: &[&str],
    iter: &CompIter,
) -> Result<Ty, Diagnostic> {
    match iter {
        CompIter::Range { start, stop, step } => {
            check_range_operand_in(env, local_names, "start", start)?;
            check_range_operand_in(env, local_names, "stop", stop)?;
            check_range_operand_in(env, local_names, "step", step)?;
            Ok(Ty::Int)
        }
        CompIter::Name(name) => {
            let base_ty = lookup_bound_name(env, local_names, name)?;
            match base_ty {
                Ty::List(elem_ty) => Ok(*elem_ty),
                Ty::Dict(kv) => Ok(kv.0),
                Ty::Set(elem_ty) | Ty::FrozenSet(elem_ty) => Ok(*elem_ty),
                other => Err(Diagnostic::error(
                    "T0033",
                    format!(
                        "`{}` cannot be iterated with `for ... in ...` (only list[T]/dict[K, V]/set[T]/frozenset[T] supports this)",
                        other.name()
                    ),
                    Span::new(0, 0),
                )),
            }
        }
        CompIter::Iterable(iterable) => match infer_expr_in(env, local_names, iterable)? {
            Ty::Object => Ok(Ty::Object),
            other => Err(non_object_iterable(&other)),
        },
    }
}

/// The `C0001` for a comprehension whose iterable expression is not a
/// CPython object (Part 1 of #1255). Before #1255 the HIR refused every
/// such shape by syntax; the type is what decides it now, and a native
/// container must still be bound to a name first.
fn non_object_iterable(ty: &Ty) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "only `range(...)`, a bare name or a CPython object is supported so far as a comprehension's iterable, got an expression of type `{}`",
            ty.name()
        ),
        Span::new(0, 0),
    )
    .with_help("bind a native container to a name first and iterate the name")
}

/// The container a list or set comprehension over a CPython object produces
/// (Part 1 of #1255): a fresh CPython `list` or `set`, typed as the opaque
/// [`Ty::Object`] (D-258). Each element is packed across the boundary, so it
/// must be packable ([`crate::foreign::is_packable_operand`]); a dict
/// comprehension over an object is not compiled yet.
fn object_comp_container_ty(
    env: &Environment,
    local_names: &[&str],
    elts: &CompElts<'_>,
) -> Result<Ty, Diagnostic> {
    let (elt, kind) = match *elts {
        CompElts::List(elt) => (elt, "list"),
        CompElts::Set(elt) => (elt, "set"),
        CompElts::Dict(..) => {
            return Err(crate::foreign::object_operation_unsupported(
                "a dict comprehension over a CPython object",
            ));
        }
    };
    let elt_ty = infer_expr_in(env, local_names, elt)?;
    if !crate::foreign::is_packable_operand(&elt_ty) {
        return Err(crate::foreign::object_operation_unsupported(&format!(
            "collecting a `{}` element into a CPython {kind}",
            elt_ty.name()
        )));
    }
    Ok(Ty::Object)
}

/// Checks `cond` and the element expressions against `env`, in which the
/// loop variable is already bound, and returns the produced container type.
/// The element gate is the one the matching display already applies (D-119
/// reuses `T0034`/`T0038`/`T0036`, identical to `ListLiteral`/`SetLiteral`/
/// `DictLiteral`'s own gates; no new diagnostic code is minted).
/// A set comprehension may also produce a `set[C]` of a hashable user class
/// (#1344), through the same `crate::set_element::check_set_element` gate a
/// set literal uses. Over a CPython object (`object_source`, Part 1 of
/// #1255) the container is a CPython object instead
/// ([`object_comp_container_ty`]).
pub(crate) fn comp_container_ty(
    env: &Environment,
    local_names: &[&str],
    cond: Option<&HirExpr>,
    elts: &CompElts<'_>,
    object_source: bool,
) -> Result<Ty, Diagnostic> {
    if let Some(cond) = cond {
        infer_expr_in(env, local_names, cond)?;
    }
    if object_source {
        return object_comp_container_ty(env, local_names, elts);
    }
    match *elts {
        CompElts::List(elt) => {
            let elt_ty = infer_expr_in(env, local_names, elt)?;
            if elt_ty != Ty::Int {
                return Err(Diagnostic::error(
                    "T0034",
                    format!(
                        "list codegen only supports `list[int]` in v0.2, got a comprehension producing `list[{}]`",
                        elt_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            Ok(Ty::List(Box::new(Ty::Int)))
        }
        CompElts::Set(elt) => {
            let elt_ty = infer_expr_in(env, local_names, elt)?;
            // #1344 (Part 2 of #1336): a set of user-class instances, gated
            // exactly like a set literal's or `.add(...)`'s element (D-255):
            // `T0054` for an unhashable class, `C0001` for a class shape
            // whose hashing is not compiled yet.
            if let Ty::Instance(_) = &elt_ty {
                crate::set_element::check_set_element(&elt_ty, env)?;
                return Ok(Ty::Set(Box::new(elt_ty)));
            }
            if elt_ty != Ty::Int {
                return Err(Diagnostic::error(
                    "T0038",
                    format!(
                        "set comprehension codegen only supports `set[int]` or a set of a user-class instance (D-122, D-255), got a comprehension producing `set[{}]`",
                        elt_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            Ok(Ty::Set(Box::new(Ty::Int)))
        }
        CompElts::Dict(key, value) => {
            let key_ty = infer_expr_in(env, local_names, key)?;
            let value_ty = infer_expr_in(env, local_names, value)?;
            if key_ty != Ty::Str || value_ty != Ty::Int {
                return Err(Diagnostic::error(
                    "T0036",
                    format!(
                        "dict codegen only supports `dict[str, int]` in v0.2, got a comprehension producing `dict[{}, {}]`",
                        key_ty.name(),
                        value_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            Ok(Ty::Dict(Box::new((Ty::Str, Ty::Int))))
        }
    }
}

/// PR-12 Task 3 (D-117): `target = <comp>` at module scope (`local_names`
/// empty) or in a function body. `var` is resolved and bound exactly like
/// `ForList`'s own loop variable *before* `cond` and the elements are
/// checked, so a reference to the loop variable inside either resolves
/// correctly; `target` is bound to the produced container type last.
pub(crate) fn check_comp_assign(
    env: &mut Environment,
    local_names: &[&str],
    target: &str,
    comp: CompView<'_>,
) -> Result<(), Diagnostic> {
    let var_ty = resolve_comp_iter(env, local_names, comp.iter)?;
    check_assignment(env, comp.var, var_ty)?;
    // A statement form never holds an iterable expression: the HIR lowers
    // that comprehension to a plain `Assign` (Part 1 of #1255).
    let container_ty = comp_container_ty(env, local_names, comp.cond, &comp.elts, false)?;
    check_assignment(env, target, container_ty)
}

impl<'a> From<&'a CompElt> for CompElts<'a> {
    fn from(elt: &'a CompElt) -> Self {
        match elt {
            CompElt::List(e) => CompElts::List(e),
            CompElt::Set(e) => CompElts::Set(e),
            CompElt::Dict { key, value } => CompElts::Dict(key, value),
        }
    }
}

/// `infer_expr_in`'s `HirExpr::Comprehension` arm (#1254, D-250). The
/// iterable resolves in the enclosing environment; `cond` and the element
/// expressions are checked in a clone with the synthesized loop variable
/// bound, so the variable never becomes a binding of the enclosing scope.
/// Narrowing and module globals carry over through the clone.
pub(crate) fn infer_comprehension(
    env: &Environment,
    local_names: &[&str],
    comp: &HirComprehension,
) -> Result<Ty, Diagnostic> {
    let var_ty = resolve_comp_iter(env, local_names, &comp.iter)?;
    let mut scoped = env.clone();
    scoped.bind(comp.var.clone(), var_ty);
    comp_container_ty(
        &scoped,
        local_names,
        comp.cond.as_ref(),
        &CompElts::from(&comp.elt),
        matches!(comp.iter, CompIter::Iterable(_)),
    )
}

/// The `int`-element container type a comprehension of this kind produces:
/// `list[int]`, `set[int]` or `dict[str, int]` (D-119). A set comprehension of
/// user-class instances (#1344) is typed by [`comp_container_ty`] instead.
pub(crate) fn comp_container_of(elt: &CompElt) -> Ty {
    match elt {
        CompElt::List(_) => Ty::List(Box::new(Ty::Int)),
        CompElt::Set(_) => Ty::Set(Box::new(Ty::Int)),
        CompElt::Dict { .. } => Ty::Dict(Box::new((Ty::Str, Ty::Int))),
    }
}

/// The `help` of [`inferred_set_return_limit`]'s `C0001`.
const INFERRED_SET_RETURN_HELP: &str = "annotate the helper's return, for example `-> set[C]`; \
     inferring it needs solver typing of a set-typed name's element in a comprehension (#1360)";

/// #1344 (Part 2 of #1336): the honest `C0001` for an unannotated private
/// helper whose solver-inferred return is `set[int]`/`frozenset[int]` while
/// its body returns a `set[C]`/`frozenset[C]` of user-class instances.
///
/// The solver types a set comprehension's container from its element term
/// (`crate::constraints::set_comp`), and falls back to `set[int]` when it
/// has no term for the element: the loop variable of a comprehension over a
/// set-typed name (#1360). A class-constructor element has a term since
/// #1342 (`crate::constraints::constructor_call`). The
/// check phase does type that element, so without this relabel the program
/// would meet a false `T0022` "expected return type `set[int]`". Returns
/// `None` for every other mismatch, which keeps its own diagnostic; the
/// caller consults it only for an inferred return (`Environment::
/// return_inferred`), so an annotated `-> set[int]` keeps `T0022`.
pub(crate) fn inferred_set_return_limit(declared: &Ty, actual: &Ty) -> Option<Diagnostic> {
    let declared_int = matches!(declared, Ty::Set(e) | Ty::FrozenSet(e) if **e == Ty::Int);
    let actual_instances =
        matches!(actual, Ty::Set(e) | Ty::FrozenSet(e) if matches!(**e, Ty::Instance(_)));
    if !(declared_int && actual_instances) {
        return None;
    }
    Some(
        Diagnostic::error(
            "C0001",
            format!(
                "cannot infer an unannotated private helper's `{}` return yet",
                actual.name()
            ),
            Span::new(0, 0),
        )
        .with_help(INFERRED_SET_RETURN_HELP),
    )
}

#[cfg(test)]
mod object_tests;

#[cfg(test)]
mod tests {
    use super::inferred_set_return_limit;
    use pycc_hir::Ty;

    fn set_of(elem: Ty) -> Ty {
        Ty::Set(Box::new(elem))
    }

    #[test]
    fn an_inferred_int_set_return_holding_instances_is_c0001() {
        let r = Ty::Instance(Box::new("R".to_string()));
        for declared in [set_of(Ty::Int), Ty::FrozenSet(Box::new(Ty::Int))] {
            for actual in [set_of(r.clone()), Ty::FrozenSet(Box::new(r.clone()))] {
                let diag = inferred_set_return_limit(&declared, &actual)
                    .expect("an instance set against an inferred int set is relabelled");
                assert_eq!(diag.code, "C0001");
                assert!(diag.message.contains(&actual.name()), "{}", diag.message);
                let help = diag.help.as_deref().unwrap_or_default();
                assert!(help.contains("#1360") && !help.contains("#1342"), "{help}");
            }
        }
    }

    #[test]
    fn every_other_mismatch_keeps_its_own_diagnostic() {
        let r = Ty::Instance(Box::new("R".to_string()));
        for (declared, actual) in [
            (set_of(Ty::Str), set_of(r.clone())),
            (set_of(Ty::Int), set_of(Ty::Str)),
            (Ty::Int, set_of(r.clone())),
            (set_of(Ty::Int), r),
        ] {
            assert!(inferred_set_return_limit(&declared, &actual).is_none());
        }
    }
}
