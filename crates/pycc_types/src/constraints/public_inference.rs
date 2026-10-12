//! Strict mode's inferred public functions.
//!
//! A native build (not `--ext`) links its whole import closure, so every
//! caller of an unannotated public module-level function is in the program:
//! `pycc_hir` lowers it like a private helper (`Ty::Infer` parameters and
//! return) and the solver infers *one* signature from its uses, exactly as
//! non-strict mode already does. Where non-strict mode reads a type the
//! solver cannot pin as an object, strict mode refuses the function with the
//! `T0001` a missing annotation always had: a parameter no call site types,
//! call sites that disagree on a parameter's type, or a return the body does
//! not pin.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirItem, HirModule};

/// Whether a failure to infer `function`'s signature is reported as a
/// missing annotation (`T0001`) rather than a private helper's `T0021`:
/// strict mode and a public name. `function` is the solver's signature key;
/// a method's key is `Class.method`, whose method part decides.
pub(crate) fn reports_missing_annotation(function: &str) -> bool {
    !crate::non_strict::is_non_strict() && pycc_hir::is_public_name(source_name(function))
}

/// The name a `T0001` message spells: a method's own name, as `pycc_hir`'s
/// annotation check spells it.
fn source_name(function: &str) -> &str {
    function.rsplit('.').next().unwrap_or(function)
}

/// `T0001` for a parameter of `function` whose type inference could not pin.
pub(crate) fn parameter_needs_annotation(function: &str, param: &str) -> Diagnostic {
    let name = source_name(function);
    Diagnostic::error(
        "T0001",
        format!("parameter `{param}` of public function `{name}` needs a type annotation"),
        Span::new(0, 0),
    )
    .with_help(format!("add a type annotation to parameter `{param}`"))
}

/// `T0001` for a parameter whose call sites pass values of different types:
/// one inferred signature cannot serve them all.
pub(crate) fn parameter_conflict(function: &str, param: &str, conflict: &Diagnostic) -> Diagnostic {
    let mut diagnostic = parameter_needs_annotation(function, param);
    diagnostic.span = conflict.span;
    diagnostic.with_help(format!(
        "its call sites pass values of different types ({}); add a type annotation to \
         parameter `{param}`",
        conflict.message
    ))
}

/// `T0001` for a return type inference could not pin.
pub(crate) fn return_needs_annotation(function: &str) -> Diagnostic {
    let name = source_name(function);
    Diagnostic::error(
        "T0001",
        format!("public function `{name}` needs a return type annotation"),
        Span::new(0, 0),
    )
    .with_help(format!("add a return type annotation to `{name}`"))
}

/// Places `diagnostic` on the `def` of `function` (a module-level function
/// of `hir`), as a missing annotation is placed.
pub(crate) fn at_function(hir: &HirModule, function: &str, diagnostic: Diagnostic) -> Diagnostic {
    match hir
        .items
        .iter()
        .position(|item| matches!(item, HirItem::Function { name, .. } if name == function))
    {
        Some(index) => crate::locate::place_item(diagnostic, index),
        None => diagnostic,
    }
}
