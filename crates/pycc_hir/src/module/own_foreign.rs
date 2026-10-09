//! Issue #1482: whole-module object-receiver admission for a module's own
//! foreign imports.
//!
//! Before #1482 a module's own foreign import admitted object receivers
//! (#1095) only from its statement down, so a `def` written above
//! `import gc` that calls `gc.garbage.append(1)` was refused, although
//! CPython reads `gc` when the function runs, after the import. The
//! admission is now decided once, before `lower_module`'s item loop, by
//! [`binds_foreign_import`], which replays the loop's import lowering.

use crate::import::{FuturePosition, ResolvedImports, future_prologue_len, lower_block_imports};
use crate::{ForeignImportSite, ImportBinding, lower_import_stmt};
use pycc_ast::ModModule;

/// Whether the module's own body binds a foreign import that lowering will
/// bind: at top level, or nested in a module-level `if`/`try` outside a
/// `TYPE_CHECKING` body.
///
/// It replays `ModuleState::imports` in source order, starting from `seed`
/// (the table the item loop starts from), through the same two functions
/// `lower_top_level_item` calls -- [`lower_import_stmt`] for a top-level
/// statement and [`lower_block_imports`] for a module-level block -- so an
/// aliased `if t.TYPE_CHECKING:` guard is recognized exactly as the item
/// loop recognizes it at that statement (a later `import math as t` does
/// not change it). In a module that lowers, it therefore answers `true`
/// exactly when some statement of the loop binds a foreign import. In a
/// module that fails it can admit where the loop never reaches an import,
/// which only changes which diagnostics a failing module reports.
pub(crate) fn binds_foreign_import(
    module: &ModModule,
    resolved: &ResolvedImports<'_>,
    seed: &[ImportBinding],
) -> bool {
    let prologue_len = future_prologue_len(&module.body);
    let mut imports = seed.to_vec();
    for (index, stmt) in module.body.iter().enumerate() {
        let position = if index < prologue_len {
            FuturePosition::Prologue
        } else {
            FuturePosition::Body
        };
        // `site` never changes whether a binding is foreign.
        let bindings = match lower_import_stmt(stmt, resolved, position, ForeignImportSite::Item(0))
        {
            Ok(Some(lowered)) => lowered.bindings,
            Ok(None) => lower_block_imports(stmt, resolved, &imports).bindings,
            // The item loop refuses this statement, so the module fails.
            Err(_) => continue,
        };
        if bindings
            .iter()
            .any(|binding| matches!(binding, ImportBinding::Foreign { .. }))
        {
            return true;
        }
        imports.extend(bindings);
    }
    false
}
