//! #1331 (Part 1 of #1327): the `hash(...)` builtin call, extended by #1335
//! (Part 1 of #1332) to a user-class instance.
//!
//! `hash(x)` is admitted for an `int`, a `bool`, a tuple whose elements are
//! all `int`/`bool`, and an instance whose class resolves to the identity
//! hash or to a plain `def __hash__(self)` returning `int`/`bool`; it always
//! yields `int`. A `list`, `dict` or `set` argument, and an instance whose
//! class binds `__eq__` without `__hash__`, is `T0021` "unhashable type",
//! the `TypeError` CPython raises for it reported statically (the `len(5)`
//! precedent). Every other argument -- `str`, `float`, a float-element tuple,
//! `None`, `frozenset[int]`, an instance `pycc_hir::resolve_instance_hash`
//! refuses -- is hashable in CPython but not yet here, so it is an honest
//! `C0001`.
//!
//! The call is checked on both of this crate's paths: the public-body path
//! (`crate::expr::infer_expr_in`) and the constraint path for unannotated
//! private helpers (`crate::constraints`). Both place the arm *after* every
//! user-binding lookup, beside `frozenset`'s -- a program's own `def hash`
//! or `class hash` keeps its meaning -- and both also yield to a stdlib
//! module alias spelled `hash` (`import math as hash`), which no user table
//! records. The non-instance rule is one function, [`check_value`], both
//! paths call, so they cannot drift the way `len`'s once did (#1098). The
//! instance verdict is the exception: it needs the class table, which the
//! constraint path's `ConstraintEnvironment` does not carry, so that path
//! admits a known instance leniently and the check phase -- which re-runs
//! every private helper's body once its types are solved -- delivers the
//! verdict ([`check_call`]).
//!
//! In its own module because `expr.rs`, `constraints.rs` and `lib.rs` are all
//! past AGENTS.md's ~1,000-line decomposition threshold.

use crate::env::Environment;
use crate::set_element::check_hash_method;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirClassDef, InstanceHash, Ty, resolve_instance_hash};
use std::collections::HashMap;

/// The builtin's spelling.
pub(crate) const HASH: &str = "hash";

/// `T0021` for a call with other than exactly one argument.
fn wrong_arity(count: usize) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!("`hash` expects exactly 1 argument, got {count}"),
        Span::new(0, 0),
    )
    .with_help("pass exactly 1 argument")
}

/// Whether `ty` is a type `hash()` compiles today.
fn is_admitted(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Bool => true,
        Ty::Tuple(elements) => elements.iter().all(|e| matches!(e, Ty::Int | Ty::Bool)),
        _ => false,
    }
}

/// The refusal for an argument whose type is not admitted.
fn refusal(ty: &Ty) -> Diagnostic {
    if matches!(ty, Ty::List(_) | Ty::Dict(_) | Ty::Set(_)) {
        return Diagnostic::error(
            "T0021",
            format!("unhashable type: `{}`", ty.name()),
            Span::new(0, 0),
        )
        .with_help("CPython raises `TypeError` here; hash an `int`, `bool` or tuple instead");
    }
    not_implemented(
        ty,
        "`hash()` currently accepts an `int`, a `bool`, a tuple of them, or a class instance"
            .to_string(),
    )
}

/// The `C0001` for a hashable argument pycc does not compile yet.
fn not_implemented(ty: &Ty, help: String) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "`hash()` of `{}` is valid Python but not implemented yet",
            ty.name()
        ),
        Span::new(0, 0),
    )
    .with_help(help)
}

/// Checks `hash()` of an instance of `class` against the class table and
/// the function signatures: the verdict of
/// [`pycc_hir::resolve_instance_hash`], then, for a user `__hash__`, its
/// signature.
fn check_instance(
    ty: &Ty,
    class: &str,
    classes: &HashMap<String, HirClassDef>,
    functions: &HashMap<String, (Vec<Ty>, Ty)>,
) -> Result<Ty, Diagnostic> {
    match resolve_instance_hash(class, classes) {
        InstanceHash::Identity => Ok(Ty::Int),
        InstanceHash::Method(mangled) => {
            // #1343: the signature rule is shared with a set element.
            check_hash_method(&mangled, functions)
                .map_err(|refusal| refusal.into_diagnostic(|help| not_implemented(ty, help)))?;
            Ok(Ty::Int)
        }
        InstanceHash::Unhashable { class: binder } => Err(Diagnostic::error(
            "T0021",
            format!("unhashable type: `{}`", ty.name()),
            Span::new(0, 0),
        )
        .with_help(format!(
            "CPython raises `TypeError` here: `{binder}` defines `__eq__` without `__hash__`, \
             so its instances are unhashable; define `__hash__` on `{binder}`"
        ))),
        InstanceHash::Unsupported(refusal) => Err(not_implemented(ty, refusal.help())),
    }
}

/// Checks `hash()` of a known non-instance argument: the rule both paths
/// share.
fn check_value(arg: &Ty) -> Result<Ty, Diagnostic> {
    if is_admitted(arg) {
        Ok(Ty::Int)
    } else {
        Err(refusal(arg))
    }
}

/// Checks a `hash(...)` call whose argument types are all known (the
/// public-body path, which also re-checks every private helper once the
/// solver has typed it) and returns `int`.
pub(crate) fn check_call(arg_tys: &[Ty], env: &Environment) -> Result<Ty, Diagnostic> {
    match arg_tys {
        [arg @ Ty::Instance(class)] => check_instance(arg, class, &env.classes, &env.functions),
        [arg] => check_value(arg),
        _ => Err(wrong_arity(arg_tys.len())),
    }
}

/// The constraint-path twin of [`check_call`]. An argument whose term is
/// unresolved, or still mentions an inference placeholder, is admitted,
/// because the result never depends on it: the final check pass
/// (`check_call` through `infer_expr_in`) validates it once it is known --
/// the lenient-until-known pattern `float` and `frozenset` already use. A
/// known instance is admitted the same way, for the check pass to deliver
/// its class verdict (see this module's documentation).
pub(crate) fn check_call_terms<V>(arg_terms: &[Option<Result<Ty, V>>]) -> Result<Ty, Diagnostic> {
    match arg_terms {
        [Some(Ok(Ty::Instance(_)))] => Ok(Ty::Int),
        [Some(Ok(arg))] if !crate::empty_container::contains_infer(arg) => check_value(arg),
        [_] => Ok(Ty::Int),
        _ => Err(wrong_arity(arg_terms.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Environment {
        Environment::default()
    }

    fn tuple(elements: Vec<Ty>) -> Ty {
        Ty::Tuple(Box::new(elements))
    }

    #[test]
    fn int_bool_and_int_bool_tuples_yield_int() {
        for arg in [
            Ty::Int,
            Ty::Bool,
            tuple(vec![Ty::Int]),
            tuple(vec![Ty::Bool, Ty::Int, Ty::Bool]),
        ] {
            assert_eq!(check_call(&[arg], &env()).unwrap(), Ty::Int);
        }
    }

    #[test]
    fn a_container_is_t0021_unhashable() {
        for arg in [
            Ty::List(Box::new(Ty::Int)),
            Ty::Dict(Box::new((Ty::Int, Ty::Int))),
            Ty::Set(Box::new(Ty::Int)),
        ] {
            let err = check_call(std::slice::from_ref(&arg), &env()).unwrap_err();
            assert_eq!(err.code, "T0021");
            assert_eq!(err.message, format!("unhashable type: `{}`", arg.name()));
        }
    }

    #[test]
    fn a_hashable_but_unimplemented_argument_is_c0001() {
        for arg in [
            Ty::Str,
            Ty::Float,
            Ty::None,
            tuple(vec![Ty::Float, Ty::Int]),
            Ty::FrozenSet(Box::new(Ty::Int)),
        ] {
            let err = check_call(std::slice::from_ref(&arg), &env()).unwrap_err();
            assert_eq!(err.code, "C0001");
            assert_eq!(
                err.message,
                format!(
                    "`hash()` of `{}` is valid Python but not implemented yet",
                    arg.name()
                )
            );
        }
    }

    #[test]
    fn any_other_arity_is_t0021() {
        let err = check_call(&[], &env()).unwrap_err();
        assert_eq!(err.message, "`hash` expects exactly 1 argument, got 0");
        let err = check_call(&[Ty::Int, Ty::Int], &env()).unwrap_err();
        assert_eq!(err.code, "T0021");
        assert_eq!(err.message, "`hash` expects exactly 1 argument, got 2");
    }

    #[test]
    fn the_constraint_twin_is_lenient_only_until_the_type_is_known() {
        assert_eq!(check_call_terms::<usize>(&[None]).unwrap(), Ty::Int);
        assert_eq!(check_call_terms::<usize>(&[Some(Err(3))]).unwrap(), Ty::Int);
        assert_eq!(
            check_call_terms::<usize>(&[Some(Ok(tuple(vec![Ty::Int, Ty::Infer])))]).unwrap(),
            Ty::Int
        );
        assert_eq!(
            check_call_terms::<usize>(&[Some(Ok(Ty::Bool))]).unwrap(),
            Ty::Int
        );
        let err = check_call_terms::<usize>(&[Some(Ok(Ty::Str))]).unwrap_err();
        assert_eq!(err.code, "C0001");
        let empty: [Option<Result<Ty, usize>>; 0] = [];
        let err = check_call_terms(&empty).unwrap_err();
        assert_eq!(err.message, "`hash` expects exactly 1 argument, got 0");
    }

    /// Lowers and checks `source`, returning the first diagnostic, if any.
    fn check_source(source: &str) -> Option<Diagnostic> {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
        crate::check_and_resolve(&hir).err()
    }

    const R: &str = "class R:\n    def __init__(self) -> None:\n        self.x = 1\n";

    #[test]
    fn a_method_signature_starts_with_self() {
        // Pins the receiver the arity rule counts: a `def __hash__(self)`
        // is registered with exactly one parameter.
        let module = pycc_parser::parse(&format!(
            "{R}\n    def __hash__(self) -> int:\n        return 3\n"
        ))
        .expect("parses");
        let hir = pycc_hir::lower_checked(&module).expect("lowers");
        let params = hir
            .items
            .iter()
            .find_map(|item| match item {
                pycc_hir::HirItem::Function { name, params, .. } if name == "R.__hash__" => {
                    Some(params.clone())
                }
                _ => None,
            })
            .expect("the method is an item");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].0, "self");
    }

    #[test]
    fn an_identity_or_method_hashed_instance_yields_int() {
        assert!(check_source(&format!("{R}\n\nprint(hash(R()))\n")).is_none());
        for returns in ["int", "bool"] {
            let source = format!(
                "{R}\n    def __hash__(self) -> {returns}:\n        return True\n\n\n\
                 print(hash(R()))\n"
            );
            assert!(check_source(&source).is_none(), "{returns}");
        }
    }

    #[test]
    fn an_eq_only_class_is_t0021_unhashable() {
        let source = "class A:\n    def __eq__(self, other: int) -> bool:\n        return True\n\
                      \n\nclass B(A):\n    pass\n\n\nprint(hash(B()))\n";
        let err = check_source(source).expect("refused");
        assert_eq!(err.code, "T0021");
        assert_eq!(err.message, "unhashable type: `B`");
        assert_eq!(
            err.help.as_deref(),
            Some(
                "CPython raises `TypeError` here: `A` defines `__eq__` without `__hash__`, so \
                 its instances are unhashable; define `__hash__` on `A`"
            )
        );
    }

    #[test]
    fn a_non_integer_hash_method_is_t0021() {
        let source = format!(
            "{R}\n    def __hash__(self) -> str:\n        return \"s\"\n\n\nprint(hash(R()))\n"
        );
        let err = check_source(&source).expect("refused");
        assert_eq!(err.code, "T0021");
        assert_eq!(err.message, "`__hash__` method should return an integer");
        assert_eq!(
            err.help.as_deref(),
            Some("CPython raises `TypeError` when `R.__hash__` returns `str`; return an `int`")
        );
    }

    #[test]
    fn a_hash_method_with_extra_parameters_or_a_refused_class_is_c0001() {
        let source = format!(
            "{R}\n    def __hash__(self, y: int) -> int:\n        return y\n\n\n\
             print(hash(R()))\n"
        );
        let err = check_source(&source).expect("refused");
        assert_eq!(err.code, "C0001");
        assert_eq!(
            err.message,
            "`hash()` of `R` is valid Python but not implemented yet"
        );
        assert!(
            err.help
                .expect("help")
                .contains("`R.__hash__` takes parameters besides")
        );
        let source = format!("{R}\n    __hash__ = 1\n\n\nprint(hash(R()))\n");
        let err = check_source(&source).expect("refused");
        assert_eq!(err.code, "C0001");
        assert_eq!(
            err.help,
            Some(
                pycc_hir::HashRefusal::NotAMethod {
                    class: "R".to_string()
                }
                .help()
            )
        );
    }

    /// `check_instance` for class `R` whose `__hash__` is registered by
    /// hand as returning `returns`, independent of the method body (the
    /// source-level `-> int | None` case is pinned end to end in
    /// `tests/issue_1335_hash_instance.rs`).
    fn check_hash_returning(returns: Ty) -> Diagnostic {
        let module = pycc_parser::parse(&format!(
            "{R}\n    def __hash__(self) -> int:\n        return 3\n"
        ))
        .expect("parses");
        let hir = pycc_hir::lower_checked(&module).expect("lowers");
        let classes: HashMap<String, HirClassDef> = hir.class_defs.into_iter().collect();
        let instance = Ty::Instance(Box::new("R".to_string()));
        let mut functions = HashMap::new();
        functions.insert("R.__hash__".to_string(), (vec![instance.clone()], returns));
        check_instance(&instance, "R", &classes, &functions).unwrap_err()
    }

    #[test]
    fn an_optional_int_or_bool_hash_result_is_c0001_not_t0021() {
        for inner in [Ty::Int, Ty::Bool] {
            let err = check_hash_returning(Ty::Optional(Box::new(inner)));
            assert_eq!(err.code, "C0001");
            assert_eq!(
                err.message,
                "`hash()` of `R` is valid Python but not implemented yet"
            );
            let help = err.help.expect("help");
            assert!(help.contains("`R.__hash__` returns `"), "{help}");
            assert!(help.contains("returning `int` or `bool`"), "{help}");
        }
    }

    #[test]
    fn an_optional_float_hash_result_stays_t0021() {
        let err = check_hash_returning(Ty::Optional(Box::new(Ty::Float)));
        assert_eq!(err.code, "T0021");
        assert_eq!(err.message, "`__hash__` method should return an integer");
    }

    #[test]
    fn the_constraint_twin_admits_a_known_instance_for_the_check_phase() {
        let instance = Ty::Instance(Box::new("R".to_string()));
        assert_eq!(
            check_call_terms::<usize>(&[Some(Ok(instance))]).unwrap(),
            Ty::Int
        );
    }
}
