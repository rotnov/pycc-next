//! Part 1 of #1319: the `frozenset(...)` builtin call and the
//! string-conversion refusal shared by `set[T]` and `frozenset[T]`.
//!
//! `frozenset[int]` is `set[int]`'s immutable sibling (`Ty::FrozenSet`). It
//! shares `set[int]`'s run-time representation, so immutability exists only
//! here, in the type checker: `.add()` on a frozenset is refused by
//! `HirExpr::SetAdd`'s own `let Ty::Set(..) else` arm, and a `set[int]` is not
//! assignable to `frozenset[int]` (or back) because `is_assignable` is
//! structural. The only conversion is an explicit `frozenset(x)`.
//!
//! #1343 widens the admitted sources to a `set`/`frozenset` of a user-class
//! instance; the result keeps the source's element type ([`frozenset_of`]).
//! On the constraint path a source still unresolved when the call is checked
//! is deferred to `constraints::set_comp`'s container defaults.
//!
//! The call is checked on both of this crate's paths: the public-body path
//! (`crate::expr::infer_expr_in`) and the constraint path for unannotated
//! private helpers (`crate::constraints`). Both place the arm *after* every
//! user-binding lookup -- a program's own `def frozenset` or `class
//! frozenset` keeps its meaning -- and both call into this module, so the
//! two paths cannot drift the way `len`'s once did (#1098).
//!
//! In its own module because `expr.rs`, `constraints.rs` and `lib.rs` are all
//! past AGENTS.md's ~1,000-line decomposition threshold.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::Ty;

/// The builtin's spelling.
pub(crate) const FROZENSET: &str = "frozenset";

/// The one compiled frozenset type, `frozenset[int]` (D-122).
pub(crate) fn frozenset_int() -> Ty {
    Ty::FrozenSet(Box::new(Ty::Int))
}

/// Whether `ty` is a value `frozenset(...)` can copy its elements from: a
/// `set[int]`, a `frozenset[int]` or a `list[int]`, or (#1343) a set or
/// frozenset of a user-class instance. `list[R]` is `T0034` already.
fn is_admitted_source(ty: &Ty) -> bool {
    match ty {
        Ty::Set(element) | Ty::FrozenSet(element) => {
            matches!(**element, Ty::Int | Ty::Instance(_))
        }
        Ty::List(element) => **element == Ty::Int,
        _ => false,
    }
}

/// The frozenset `frozenset(source)` produces: the element type of a set or
/// frozenset source (the stored words and hashes are copied, #1343), and
/// `frozenset[int]` otherwise.
fn frozenset_of(source: &Ty) -> Ty {
    match source {
        Ty::Set(element) | Ty::FrozenSet(element) => Ty::FrozenSet(element.clone()),
        _ => frozenset_int(),
    }
}

/// The admitted sources, for both help lines.
const SOURCES_HELP: &str = "a `set[int]`, `frozenset[int]` or `list[int]` value, or a set or \
                            frozenset of a hashable user class";

/// `T0021` for a call with more than one argument.
fn wrong_arity(count: usize) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!("`frozenset` expects at most 1 argument, got {count}"),
        Span::new(0, 0),
    )
    .with_help(format!("pass no argument, or one {SOURCES_HELP}"))
}

/// `T0021` for an argument whose type is not an admitted source.
fn wrong_source(ty: &Ty) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!(
            "`frozenset()` argument must be a `set[int]`, `frozenset[int]` or `list[int]` here, got `{}`",
            ty.name()
        ),
        Span::new(0, 0),
    )
    .with_help(format!("pass {SOURCES_HELP}"))
}

/// Checks a `frozenset(...)` call whose argument types are all known (the
/// public-body path) and returns the frozenset it produces.
pub(crate) fn check_call(arg_tys: &[Ty]) -> Result<Ty, Diagnostic> {
    match arg_tys {
        [] => Ok(frozenset_int()),
        [source] if is_admitted_source(source) => Ok(frozenset_of(source)),
        [source] => Err(wrong_source(source)),
        _ => Err(wrong_arity(arg_tys.len())),
    }
}

/// The constraint-path twin of [`check_call`], returning the result's term.
/// An argument whose term is still unresolved is admitted -- the final check
/// pass (`check_call` through `infer_expr_in`) validates it once it is known,
/// the lenient-until-known pattern `float` already uses -- and its result is
/// a fresh term the solver defaults once the argument settles (#1343,
/// `constraints::set_comp`), since the result's element is the argument's.
pub(crate) fn check_call_terms(
    arg_terms: &[Option<Result<Ty, usize>>],
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut crate::DeferredConstraints,
) -> Result<Result<Ty, usize>, Diagnostic> {
    match arg_terms {
        [] => Ok(Ok(frozenset_int())),
        [Some(Ok(source))] if !is_admitted_source(source) => Err(wrong_source(source)),
        [Some(Ok(source))] => Ok(Ok(frozenset_of(source))),
        [Some(Err(var))] => Ok(match crate::resolved_term(Err(*var), parents, concrete) {
            Some(source) => Ok(frozenset_of(&source)),
            None => crate::deferred_frozenset(Some(*var), parents, concrete, deferred),
        }),
        [None] => Ok(crate::deferred_frozenset(None, parents, concrete, deferred)),
        _ => Err(wrong_arity(arg_terms.len())),
    }
}

/// `C0001` for handing a `set[T]` or `frozenset[T]` value to a string
/// conversion (`print`, an f-string interpolation). Refused rather than
/// rendered: pycc keeps a set's insertion order (D-123), so it cannot
/// promise CPython's element order, and before this refusal `print({1})`
/// passed `pycc check` and then panicked in `pycc_codegen`'s `to_str`.
pub(crate) fn unrenderable_set(ty: &Ty) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "printing or formatting a `{}` is not supported yet",
            ty.name()
        ),
        Span::new(0, 0),
    )
    .with_help("print an order-insensitive fact instead, such as `len(...)`")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> Ty {
        Ty::Set(Box::new(Ty::Int))
    }

    #[test]
    fn every_admitted_source_and_the_empty_call_yield_frozenset_int() {
        assert_eq!(check_call(&[]).unwrap(), frozenset_int());
        for source in [set(), frozenset_int(), Ty::List(Box::new(Ty::Int))] {
            assert_eq!(check_call(&[source]).unwrap(), frozenset_int());
        }
    }

    #[test]
    fn a_non_int_or_non_container_source_is_t0021() {
        for source in [Ty::Int, Ty::Str, Ty::List(Box::new(Ty::Infer))] {
            let err = check_call(std::slice::from_ref(&source)).unwrap_err();
            assert_eq!(err.code, "T0021");
            assert!(err.message.contains(&format!("got `{}`", source.name())));
        }
    }

    #[test]
    fn two_arguments_are_t0021() {
        let err = check_call(&[set(), set()]).unwrap_err();
        assert_eq!(err.code, "T0021");
        assert_eq!(err.message, "`frozenset` expects at most 1 argument, got 2");
    }

    #[test]
    fn the_constraint_twin_is_lenient_only_for_an_unresolved_term() {
        let mut parents = vec![0];
        let mut concrete: Vec<Option<Ty>> = vec![None];
        let mut deferred = crate::DeferredConstraints::default();
        let mut call = |terms: &[Option<Result<Ty, usize>>]| {
            check_call_terms(terms, &mut parents, &mut concrete, &mut deferred)
        };
        assert_eq!(call(&[]).unwrap(), Ok(frozenset_int()));
        // An unresolved argument (variable 0) and an opaque one each get a
        // fresh, deferred result term (variables 1 and 2).
        assert_eq!(call(&[Some(Err(0))]).unwrap(), Err(1));
        assert_eq!(call(&[None]).unwrap(), Err(2));
        assert_eq!(call(&[Some(Ok(set()))]).unwrap(), Ok(frozenset_int()));
        let err = call(&[Some(Ok(Ty::Int))]).unwrap_err();
        assert_eq!(err.code, "T0021");
        let err = call(&[None, None]).unwrap_err();
        assert_eq!(err.message, "`frozenset` expects at most 1 argument, got 2");
        assert_eq!(
            deferred.container_defaults,
            vec![
                crate::ContainerDefault::FrozenSetOf {
                    source: Some(0),
                    result: 1
                },
                crate::ContainerDefault::FrozenSetOf {
                    source: None,
                    result: 2
                },
            ]
        );
    }

    #[test]
    fn a_resolved_source_term_gives_its_own_element() {
        let instance = Ty::Instance(Box::new("R".to_string()));
        let mut parents = vec![0, 1];
        let mut concrete = vec![
            Some(Ty::Set(Box::new(instance.clone()))),
            Some(Ty::List(Box::new(Ty::Int))),
        ];
        let mut deferred = crate::DeferredConstraints::default();
        assert_eq!(
            check_call_terms(&[Some(Err(0))], &mut parents, &mut concrete, &mut deferred).unwrap(),
            Ok(Ty::FrozenSet(Box::new(instance.clone())))
        );
        assert_eq!(
            check_call_terms(&[Some(Err(1))], &mut parents, &mut concrete, &mut deferred).unwrap(),
            Ok(frozenset_int())
        );
        assert!(deferred.container_defaults.is_empty());
    }

    #[test]
    fn a_set_or_frozenset_of_an_instance_is_an_admitted_source() {
        let instance = Ty::Instance(Box::new("R".to_string()));
        for source in [
            Ty::Set(Box::new(instance.clone())),
            Ty::FrozenSet(Box::new(instance.clone())),
        ] {
            assert_eq!(
                check_call(&[source]).unwrap(),
                Ty::FrozenSet(Box::new(instance.clone()))
            );
        }
        let err = check_call(&[Ty::List(Box::new(instance))]).unwrap_err();
        assert!(err.help.unwrap().contains("hashable user class"));
    }

    #[test]
    fn the_string_conversion_refusal_names_the_type() {
        let err = unrenderable_set(&frozenset_int());
        assert_eq!(err.code, "C0001");
        assert_eq!(
            err.message,
            "printing or formatting a `frozenset[int]` is not supported yet"
        );
    }
}
