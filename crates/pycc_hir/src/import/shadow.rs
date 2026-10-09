//! The import side table's local-name accessor and the rule that refuses a
//! second binding of a foreign import's local name (Part 1 of #1026, PR 1c
//! of #1080). Extracted from `import.rs` (#1291), which also exempted an
//! identical foreign pair from the rule.

use super::FallbackGroup;
use crate::ImportBinding;
use pycc_diag::{Diagnostic, Span};

/// The bound local name of an import, regardless of which `ImportBinding`
/// variant it is -- used by `module::lower_top_level_item`'s class-name-collision check
/// (D-068 review finding on #385) so it does not need to duplicate the
/// match on both variants at its own call site.
pub(crate) fn import_local_name(binding: &ImportBinding) -> &str {
    match binding {
        ImportBinding::Module { local_name, .. }
        | ImportBinding::Symbol { local_name, .. }
        | ImportBinding::Project { local_name, .. }
        | ImportBinding::Foreign { local_name, .. } => local_name,
    }
}

/// Refuses a module in which any other top-level binding spells the local
/// name of a foreign import (Part 1 of #1026, PR 1c of #1080), except the
/// #1485 `except ImportError` fallback of the import's own `try`
/// (`super::fallback`, `docs/RUNTIME.md`), whose `N = None` is no
/// definition and whose fallback import is an alternative to the body's.
///
/// Part 1's containment invariant is that the single producer of a
/// `Ty::Object` value is a read of a foreign binding, so refusing that read
/// refuses every derived operation. A module that binds the same name twice,
/// once foreign and once not, breaks that premise: the name's meaning then
/// depends on the position of every read, and each pass that walks the
/// module -- the check pass, the constraint solver, MIR lowering, export
/// discovery -- would have to reproduce the same positional rule
/// independently. Three review rounds on #1080 found three passes that did
/// not, most seriously `collect_exports`, which kept a `PyMethodDef` for a
/// `def` the import supersedes, so the host called a stale function where
/// CPython would hand back a module object.
///
/// Refusing the shape instead is one rule at one site, fail-closed, and
/// consistent with the cross-module case, which Part 1 already refuses.
/// Supporting either order is later work; see #1026.
///
/// The colliding binding comes from either of two tables, because a module's
/// top level binds names in both. `definition_spans` is every definition the
/// module makes, with its span, and never carries an import's own binding --
/// the import arm of `module::lower_module_item` records nothing there. So
/// `imports` is consulted as well: a second `import` statement binding the
/// same local name is invisible to `definition_spans` but supersedes the
/// foreign binding exactly as a `def` does (review round 4 on #1080, which
/// found `import json` followed by `import math as json` reaching the
/// solver and reporting a misleading receiver diagnostic against the wrong
/// statement when the name was used, and passing silently when it was not).
///
/// Two foreign imports that bind the same local name to the same module
/// (`import numpy` twice, or `import numpy` in both arms of an `if`/`else`,
/// #1291) do not shadow each other: both produce the same module object at
/// the same type, so no read depends on which one ran last. Two foreign
/// imports of one local name from *different* modules (`import a as x`,
/// `import b as x`) are refused like any other shadow, because a read would
/// then depend on the positional question the refusal exists to avoid. The
/// object is the module *and*, for a from-import, the attribute (#1278):
/// `import copy` followed by `from copy import copy` binds two different
/// objects to `copy`, so it is refused too, while `from copy import copy`
/// written twice is the identical pair. The module compared is the one
/// bound ([`crate::foreign_bound_module`]), not the one imported (#1381):
/// `import os` then `import os.path`, or `import a.b, a.c`, bind the one
/// module `os` (`a`) to one name, so they are identical pairs, and each
/// statement still runs its own import.
///
/// The one other exemption is the optional-dependency fallback (#1485):
/// `try: from A import N` / `except ImportError: N = None` (or `from B
/// import N`). `fallback_groups` holds every such name with the span and
/// arm of each of its `try`'s import statements
/// ([`super::fallback_groups`]). The handler's `N = None` is never in
/// `definition_spans` (`module::lower_top_level_item` skips a group name),
/// and two foreign imports of `N` do not shadow each other when one group
/// holds both *in different arms*: the body and a handler, or two
/// handlers, are alternatives, so only one of them binds on any run. Two
/// different imports in the same arm are still the sequential rebinding
/// refused above, and so is an import of `N` anywhere outside the group's
/// `try`, because the exemption is keyed by statement span, not by name.
/// The invariant the rule protects survives: every runtime binding of a
/// fallback name is a foreign import or a boxed `None` written in the same
/// `try`, so its module global slot is `object` on every path, and the
/// body's import keeps the name in `program::link`'s foreign table.
///
/// At most one diagnostic per name, so a duplicated import reports once. A
/// definition's span is preferred over the import's when both exist: it is
/// the statement that is unusual, the import being ordinary on its own.
pub(crate) fn reject_shadowed_foreign_imports(
    imports: &[ImportBinding],
    definition_spans: &[(String, Span)],
    fallback_groups: &[FallbackGroup],
) -> Vec<Diagnostic> {
    let mut reported: Vec<&str> = Vec::new();
    let mut diagnostics = Vec::new();
    for (index, binding) in imports.iter().enumerate() {
        let ImportBinding::Foreign {
            local_name,
            module_path,
            from,
            span,
            ..
        } = binding
        else {
            continue;
        };
        if reported.contains(&local_name.as_str()) {
            continue;
        }
        let definition = definition_spans
            .iter()
            .find(|(name, _)| name == local_name)
            .map(|(_, span)| *span);
        let shadowed_by_import = imports.iter().enumerate().any(|(other, candidate)| {
            other != index
                && import_local_name(candidate) == local_name
                && !matches!(candidate, ImportBinding::Foreign { span: other_span, .. }
                    if fallback_groups
                        .iter()
                        .any(|group| group.are_alternatives(local_name, *span, *other_span)))
                && !matches!(
                    candidate,
                    ImportBinding::Foreign { local_name: other_name, module_path: other_path, from: other_from, .. }
                        if crate::foreign_bound_module(other_name, other_path, other_from.as_ref())
                            == crate::foreign_bound_module(local_name, module_path, from.as_ref())
                            && other_from.as_ref().map(|f| (&f.name, f.level))
                                == from.as_ref().map(|f| (&f.name, f.level))
                )
        });
        let Some(span) = definition.or(shadowed_by_import.then_some(*span)) else {
            continue;
        };
        reported.push(local_name.as_str());
        // `C0001` by hand rather than through `unsupported`: the span is
        // already a `Span` recorded during lowering, not an AST
        // `TextRange`, and that helper takes only the range shape.
        diagnostics.push(Diagnostic::error(
            "C0001",
            format!(
                "`{local_name}` is bound both by a foreign `import` and by another top-level \
                 statement in this module; shadowing a foreign import is not supported yet"
            ),
            span,
        ));
    }
    diagnostics
}
