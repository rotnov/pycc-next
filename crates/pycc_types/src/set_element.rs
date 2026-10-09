//! #1343 (Part 1 of #1336): whether a value may be inserted into a set.
//!
//! A `set[T]`/`frozenset[T]` element is an `int` or (#1343) an instance of a
//! hashable user class. The annotation gate (`pycc_hir::check_container_ty`,
//! `T0038`) has no class table, so it admits every instance; this module
//! delivers the class verdict at the three insertion sites, a set literal,
//! `.add(...)` and (#1344) a set comprehension's element, from the same
//! `pycc_hir` resolvers MIR lowers with:
//!
//! - [`pycc_hir::resolve_instance_hash`]: a class binding `__eq__` without
//!   `__hash__` is `T0054`, CPython's own `TypeError` reported statically;
//!   any other refusal is `C0001` with the refusal's `help`.
//! - [`pycc_hir::resolve_instance_eq`]: a refusal is `C0001`.
//! - [`check_hash_method`], shared with `hash()`, for a user `__hash__`.
//! - the `__eq__` signature rule: exactly `(self, other: K) -> bool`, with
//!   `K` the element class or a class in its MRO.
//!
//! `docs/TYPE_SYSTEM.md`'s set section and the #1343 decision record are the
//! contract. In its own module because `expr.rs` and `lib.rs` are past
//! AGENTS.md's ~1,000-line decomposition threshold.

use crate::env::Environment;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{InstanceEq, InstanceHash, Ty, resolve_instance_eq, resolve_instance_hash};
use std::collections::HashMap;

/// Why a user `__hash__` method is not compiled, with the `help` line. Each
/// caller wraps it in its own headline ([`HashMethodRefusal::into_diagnostic`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HashMethodRefusal {
    /// A signature pycc does not compile yet: `C0001`.
    NotCompiled(String),
    /// A return CPython rejects with `TypeError`: `T0021`.
    NotAnInteger(String),
}

impl HashMethodRefusal {
    /// The diagnostic: `T0021` with CPython's own message, or the caller's
    /// `C0001` built by `not_implemented` from the help line.
    pub(crate) fn into_diagnostic(
        self,
        not_implemented: impl FnOnce(String) -> Diagnostic,
    ) -> Diagnostic {
        match self {
            HashMethodRefusal::NotCompiled(help) => not_implemented(help),
            HashMethodRefusal::NotAnInteger(help) => Diagnostic::error(
                "T0021",
                "`__hash__` method should return an integer",
                Span::new(0, 0),
            )
            .with_help(help),
        }
    }
}

/// Checks the signature of the user `__hash__` method `mangled`: exactly
/// `(self)`, returning `int` or `bool`.
///
/// # Panics
/// When `mangled` is not registered; a class method always is.
pub(crate) fn check_hash_method(
    mangled: &str,
    functions: &HashMap<String, (Vec<Ty>, Ty)>,
) -> Result<(), HashMethodRefusal> {
    // A method's registered parameters start with `self`.
    let (params, returns) = functions
        .get(mangled)
        .expect("a class method is registered as a function");
    if params.len() != 1 {
        return Err(HashMethodRefusal::NotCompiled(format!(
            "`{mangled}` takes parameters besides `self`; pycc compiles only `def __hash__(self)`"
        )));
    }
    if let Ty::Optional(inner) = returns
        && matches!(**inner, Ty::Int | Ty::Bool)
    {
        // CPython raises only when the method returns `None` at run time, so
        // this is not a static `TypeError`. A `float | None` result raises on
        // every call and stays `T0021` below.
        return Err(HashMethodRefusal::NotCompiled(format!(
            "`{mangled}` returns `{}`; pycc compiles only a `__hash__` returning `int` or `bool`",
            returns.name()
        )));
    }
    if !matches!(returns, Ty::Int | Ty::Bool) {
        return Err(HashMethodRefusal::NotAnInteger(format!(
            "CPython raises `TypeError` when `{mangled}` returns `{}`; return an `int`",
            returns.name()
        )));
    }
    Ok(())
}

/// `C0001` for an element class pycc does not compile as a set element yet.
fn not_implemented(class: &str, help: String) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!("a set element of class `{class}` is valid Python but not implemented yet"),
        Span::new(0, 0),
    )
    .with_help(help)
}

/// `T0054`: CPython's `TypeError` for inserting an unhashable instance.
fn unhashable(class: &str, binder: &str) -> Diagnostic {
    Diagnostic::error(
        "T0054",
        format!("cannot use '{class}' as a set element (unhashable type: '{class}')"),
        Span::new(0, 0),
    )
    .with_help(format!(
        "`{binder}` defines `__eq__` without `__hash__`, so CPython sets `__hash__ = None`; \
         define `__hash__` on `{binder}`, or remove `__eq__` to hash by identity"
    ))
}

/// Checks the user `__eq__` method `mangled` of the element class `class`:
/// exactly `(self, other: K) -> bool`, `K` being `class` or a class in its
/// MRO. `is_assignable` is deliberately not used: pycc has no
/// subclass-to-base assignability, and a `class` pointer passed as `other:
/// K` mirrors how `self` already reaches an inherited method.
fn check_eq_method(class: &str, mangled: &str, env: &Environment) -> Result<(), Diagnostic> {
    let (params, returns) = env
        .functions
        .get(mangled)
        .expect("a class method is registered as a function");
    let mro = &env
        .classes
        .get(class)
        .expect("an instance's class is registered")
        .mro;
    let other_in_mro = matches!(params.as_slice(), [_, Ty::Instance(other)]
        if mro.iter().any(|name| name == other.as_str()));
    if other_in_mro && *returns == Ty::Bool {
        return Ok(());
    }
    // #1418: a `return NotImplemented` widens the method's return to the
    // CPython object whatever its annotation says, so a written `-> bool`
    // alone no longer explains this refusal. The environment holds only the
    // signature, so the note names both ways to an `object` return.
    let widened = if *returns == Ty::Object {
        "; its return is the CPython object (an `-> object` annotation, or a \
         `return NotImplemented`, which makes it so whatever the annotation says, #1418)"
    } else {
        ""
    };
    Err(not_implemented(
        class,
        format!(
            "a set compares elements through `{mangled}`, and pycc compiles it only as \
             `def __eq__(self, other: K) -> bool` with `K` being `{class}` or one of its \
             bases{widened}"
        ),
    ))
}

/// Checks that a value of type `element` may be inserted into a set: an
/// `int`, or an instance of a class whose hash and eq verdicts and method
/// signatures pycc compiles. Called by a set literal, by `.add(...)` and by
/// a set comprehension's element (`crate::comprehension::comp_container_ty`).
pub(crate) fn check_set_element(element: &Ty, env: &Environment) -> Result<(), Diagnostic> {
    let class = match element {
        Ty::Int => return Ok(()),
        Ty::Instance(class) => class.as_str(),
        // Unreachable from every caller: a literal's type passed
        // `check_container_ty` first, `.add` sees an admitted set, and a
        // comprehension calls this only for a `Ty::Instance` element.
        other => {
            let set = Ty::Set(Box::new(other.clone()));
            return Err(Diagnostic::error(
                "T0038",
                pycc_hir::set_element_message(&set, "set"),
                Span::new(0, 0),
            ));
        }
    };
    let hash = match resolve_instance_hash(class, &env.classes) {
        InstanceHash::Unhashable { class: binder } => return Err(unhashable(class, &binder)),
        InstanceHash::Unsupported(refusal) => return Err(not_implemented(class, refusal.help())),
        admitted => admitted,
    };
    let eq = match resolve_instance_eq(class, &env.classes) {
        InstanceEq::Unsupported(refusal) => return Err(not_implemented(class, refusal.help())),
        admitted => admitted,
    };
    if let InstanceHash::Method(mangled) = hash {
        check_hash_method(&mangled, &env.functions)
            .map_err(|refusal| refusal.into_diagnostic(|help| not_implemented(class, help)))?;
    }
    if let InstanceEq::Method(mangled) = eq {
        check_eq_method(class, &mangled, env)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "set_element_tests.rs"]
mod tests;
