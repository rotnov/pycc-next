//! The optional-dependency fallback of a foreign import (#1485):
//!
//! ```python
//! try:
//!     from more_itertools import product   # or `import X as product`
//! except ImportError:
//!     product = None                       # or another foreign import
//! ```
//!
//! [`fallback_groups`] finds each name such a module-level `try` binds this
//! way, so that `module::lower_top_level_item` records no definition for
//! the handler's rebinding and [`super::reject_shadowed_foreign_imports`]
//! does not report the handler's fallback import as a shadow of the body's.
//!
//! The containment argument (Part 1 of #1026) still holds for a fallback
//! name. Every runtime binding of it is either a foreign import or the
//! `None` literal, all written in the same `try` statement: the exclusivity
//! test below uses the exhaustive [`killed_names`] over every arm for every
//! non-import binding, an `except ... as name` is refused explicitly, and an
//! import outside the group is refused by the shadowing check's span-pair
//! exemption ([`FallbackGroup::are_alternatives`]). A boxed `None` (D-258's #1475
//! amendment) and a foreign object share one representation, so the module
//! global slot stays `object` on every path, and every pass that types the
//! name eagerly as `object` (`pycc_mir`'s module scope, `pycc_types`'s
//! function-body view through `foreign_object_names`) stays right. The
//! `try` body's own import keeps the name in `program::link`'s foreign
//! table, so a sibling module's definition of the name is still refused.

use crate::{HirExceptHandler, HirExpr, HirStmt, killed_names, module::is_synthesized_name};
use pycc_diag::{Diagnostic, Span};

/// The exception names whose handler catches a failed `import`: the
/// `ModuleNotFoundError` pycc raises, its base `ImportError`, and
/// `Exception`. `BaseException` is refused by type checking (`T0021`), and
/// a module-level rebinding of any of these names is refused too, so
/// matching by spelling is sound. Shared by #1290's optional-import guard
/// ([`super::block::handlers_catch_import_error`]) and the per-handler
/// fallback test here.
pub(super) const IMPORT_ERROR_CATCHERS: [&str; 3] =
    ["ImportError", "ModuleNotFoundError", "Exception"];

/// Which arm of a `try` an import statement is written in. Two imports in
/// different arms are alternatives -- at most one of them binds on any run
/// that reaches the code after the `try` -- while two in the same arm are
/// the sequential rebinding the #1291 shadowing rule refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arm {
    /// The `try` body.
    Body,
    /// The handler at this index.
    Handler(usize),
}

/// One admitted fallback name of one module-level `try` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FallbackGroup {
    /// The local name the `try` body's foreign import binds.
    pub(crate) name: String,
    /// The span of every foreign import statement of the `try` that binds
    /// `name` directly -- the body's and each qualifying handler's -- with
    /// the arm it is written in.
    pub(crate) import_spans: Vec<(Span, Arm)>,
}

impl FallbackGroup {
    /// Whether the import statements at `a` and `b` are both in this group
    /// and in different arms, so neither shadows the other.
    pub(crate) fn are_alternatives(&self, name: &str, a: Span, b: Span) -> bool {
        let arm_of = |span: Span| {
            self.import_spans
                .iter()
                .find(|(candidate, _)| *candidate == span)
                .map(|(_, arm)| *arm)
        };
        self.name == name && matches!((arm_of(a), arm_of(b)), (Some(x), Some(y)) if x != y)
    }
}

/// Whether `handler` on its own catches a failed `import`: a bare
/// `except:`, or a type list naming one of [`IMPORT_ERROR_CATCHERS`].
fn catches_import_error(handler: &HirExceptHandler) -> bool {
    handler.exc_type.as_ref().is_none_or(|names| {
        names
            .iter()
            .any(|n| IMPORT_ERROR_CATCHERS.contains(&n.as_str()))
    })
}

/// The local names bound by the foreign import statements written directly
/// in `body`, each with its statement's span, in source order.
fn direct_import_names(body: &[HirStmt]) -> Vec<(&str, Span)> {
    body.iter()
        .filter_map(|stmt| match stmt {
            HirStmt::ForeignImport { bindings, span } => Some(
                bindings
                    .iter()
                    .map(move |(local, _, _)| (local.as_str(), *span)),
            ),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The fallback groups of the module-level statements `lowered`, the
/// lowering of the one top-level statement spanning `span`.
///
/// A group is formed for a name `N` bound by a foreign import written
/// directly in the body of a `try` written directly in `lowered` when
/// some handler that itself catches a failed import
/// ([`IMPORT_ERROR_CATCHERS`] or bare) rebinds `N` by a statement written
/// directly in its body that is either `N = None` (also one target of a
/// chained `N = M = None`, which lowering writes as one `N = None` per
/// target) or a foreign import binding `N`, and no other statement of the
/// `try` binds `N`: no other non-import binding in the body, `else` or
/// `finally`, no `except ... as N`, and no other binding (a nested one, an
/// annotated assignment, a chained or unpacking assignment of a non-literal
/// value whose target lowering desugars, a `for` target) in any handler. A
/// name that fails the test forms no group and keeps the general shadowing
/// refusal.
///
/// The test sees no import statement outside the qualifying handlers:
/// [`killed_names`] records none. A stray import of `N` in the `else`, the
/// `finally` or a handler that does not catch a failed import is therefore
/// not guarded here but by the shadowing check, which exempts only a pair
/// of imports both recorded in the group's `import_spans` and in different
/// arms ([`FallbackGroup::are_alternatives`]).
///
/// # Errors
///
/// A tailored `C0001` at `span` when a qualifying handler assigns `N` a
/// value other than the `None` literal: only `None` and a fallback import
/// are supported as the fallback of a foreign import.
pub(crate) fn fallback_groups(
    lowered: &[HirStmt],
    span: Span,
) -> Result<Vec<FallbackGroup>, Diagnostic> {
    let mut groups = Vec::new();
    for stmt in lowered {
        let HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } = stmt
        else {
            continue;
        };
        let body_names = direct_import_names(body);
        let mut seen: Vec<&str> = Vec::new();
        for &(name, _) in &body_names {
            if seen.contains(&name) {
                continue;
            }
            seen.push(name);
            let others = [body.as_slice(), orelse, finalbody];
            if let Some(group) = fallback_group(name, &body_names, others, handlers, span)? {
                groups.push(group);
            }
        }
    }
    Ok(groups)
}

/// [`fallback_groups`]'s test for one name `name` of a `try` statement
/// whose body binds `body_names` by direct foreign imports. `others` is its
/// body, `else` and `finally`.
fn fallback_group(
    name: &str,
    body_names: &[(&str, Span)],
    others: [&[HirStmt]; 3],
    handlers: &[HirExceptHandler],
    span: Span,
) -> Result<Option<FallbackGroup>, Diagnostic> {
    let mut import_spans: Vec<(Span, Arm)> = body_names
        .iter()
        .filter(|(local, _)| *local == name)
        .map(|(_, import)| (*import, Arm::Body))
        .collect();
    // `killed_names` records no import, so the body's own imports are not
    // in it; anything it does record is another binding of `name`. A stray
    // import of `name` in `else`/`finally` is not in it either: the
    // shadowing check refuses that one, since its span is not in the group.
    let mut exclusive = others.iter().all(|arm| !killed_names(arm).contains(name));
    let mut rebound = false;
    for (index, handler) in handlers.iter().enumerate() {
        if handler.name.as_deref() == Some(name) {
            exclusive = false;
        }
        let qualifies = catches_import_error(handler);
        for inner in &handler.body {
            match inner {
                HirStmt::Assign { target, value } if qualifies && target == name => match value {
                    HirExpr::NoneLiteral => rebound = true,
                    _ if is_desugared_target(value) => exclusive = false,
                    _ => return Err(native_fallback(name, span)),
                },
                HirStmt::ForeignImport { bindings, span }
                    if qualifies && bindings.iter().any(|(local, _, _)| local == name) =>
                {
                    import_spans.push((*span, Arm::Handler(index)));
                    rebound = true;
                }
                other => {
                    if killed_names(std::slice::from_ref(other)).contains(name) {
                        exclusive = false;
                    }
                }
            }
        }
    }
    Ok((exclusive && rebound).then(|| FallbackGroup {
        name: name.to_string(),
        import_spans,
    }))
}

/// Whether `value` is what lowering wrote for one target of a multi-target
/// statement rather than a value the source wrote: a chained assignment's
/// temporary (`N = M = f()`, #1213) or an element of an unpacking temporary
/// (`N, M = f()`, Part 1 of #891). Such a target keeps the general
/// shadowing refusal. (`N = M = None` repeats the literal for each target
/// instead, so it is the admitted `N = None`.)
fn is_desugared_target(value: &HirExpr) -> bool {
    let temp = match value {
        HirExpr::Subscript { base, .. } => base.as_ref(),
        other => other,
    };
    matches!(temp, HirExpr::Name(name) if is_synthesized_name(name))
}

/// The tailored `C0001` for a fallback that assigns a native value.
fn native_fallback(name: &str, span: Span) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "`{name}` is rebound in an `except ImportError` handler to a value other than \
             `None`; only a `None` or a fallback `import`/`from ... import` statement is \
             supported as the fallback of a foreign import yet"
        ),
        span,
    )
}
