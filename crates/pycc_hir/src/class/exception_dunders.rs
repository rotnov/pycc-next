//! Refuses a user-defined dunder that a raisable exception class would
//! silently ignore (#1337, WI-6b).
//!
//! A value whose static type is an exception class is a runtime
//! `PyExceptionObj` that carries only a type tag and a message (Part 2 of
//! #541, D-189). `print(e)`, `f"{e}"`, the uncaught-exception line and
//! `if e:` read that object directly and never call a user `__str__` or
//! `__bool__`, so a program defining one printed the message where CPython
//! calls the override -- a silent wrong output. Until Part 3 of #541
//! materializes real exception instances, a user exception class whose MRO
//! resolves an instance-protocol dunder to a *user* class is refused here, at
//! its definition.
//!
//! A builtin exception base defines only `__str__` and `__repr__` itself
//! (`BaseException`'s); every other dunder comes from `object`, which CPython
//! places last in the MRO, so a mixin listed after the builtin base still
//! wins it. `__init__` keeps its own raisability rules (`pycc_types`'s
//! `reject_own_constructor`), and the class-level hooks
//! (`__init_subclass__`, `__class_getitem__`, `__set_name__`) run on the
//! class, not on an exception value.

use pycc_diag::Diagnostic;

use crate::exception::{builtin_exception_class_defs, is_builtin_exception_class};
use crate::{HirClassDef, unsupported};

/// Dunders a user exception class may define: the constructor and the
/// class-level hooks.
const ACCEPTED_DUNDERS: [&str; 4] = [
    "__init__",
    "__init_subclass__",
    "__class_getitem__",
    "__set_name__",
];

/// The dunders a builtin exception class defines itself.
const BUILTIN_EXCEPTION_DUNDERS: [&str; 2] = ["__str__", "__repr__"];

/// Rejects `class_def` when it is a user exception class and the first class
/// in its MRO that defines some instance-protocol dunder is a user class.
/// `defined_classes` holds every class defined before `class_def` (which is
/// itself `mro[0]` and not yet in the table).
pub(super) fn reject_ignored_exception_dunders(
    class_def: &HirClassDef,
    defined_classes: &[(String, HirClassDef)],
    range: std::ops::Range<u32>,
) -> Result<(), Diagnostic> {
    let synthetic = builtin_exception_class_defs();
    let lookup = |name: &str| {
        defined_classes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| d)
    };
    // A builtin name counts only when it names the builtin: its entry is the
    // synthetic one, or absent because the base came in through an import
    // into a module that seeds no builtins. A user class shadowing the name
    // is an ordinary class.
    let is_builtin = |name: &str| {
        is_builtin_exception_class(name)
            && lookup(name).is_none_or(|def| synthetic.iter().any(|(_, s)| s == def))
    };
    if !class_def.mro.iter().any(|name| is_builtin(name)) {
        return Ok(());
    }
    let own_dunders = |def: &HirClassDef| -> Vec<String> {
        def.methods
            .iter()
            .map(|(name, _)| name)
            .chain(def.properties.iter().map(|p| &p.name))
            .chain(def.static_methods.iter().map(|(name, _)| name))
            .chain(def.class_methods.iter().map(|(name, _)| name))
            .filter(|name| {
                name.len() > 4
                    && name.starts_with("__")
                    && name.ends_with("__")
                    && !ACCEPTED_DUNDERS.contains(&name.as_str())
            })
            .cloned()
            .collect()
    };
    let entry = |position: usize, name: &str| {
        if position == 0 {
            Some(class_def)
        } else {
            lookup(name)
        }
    };
    // Every candidate dunder, in MRO order of its first user definition.
    let mut candidates: Vec<String> = Vec::new();
    for (position, ancestor) in class_def.mro.iter().enumerate() {
        if is_builtin(ancestor) {
            continue;
        }
        for dunder in entry(position, ancestor)
            .map(own_dunders)
            .unwrap_or_default()
        {
            if !candidates.contains(&dunder) {
                candidates.push(dunder);
            }
        }
    }
    for dunder in candidates {
        let first_definer = class_def
            .mro
            .iter()
            .enumerate()
            .find(|(position, ancestor)| {
                if is_builtin(ancestor) {
                    BUILTIN_EXCEPTION_DUNDERS.contains(&dunder.as_str())
                } else {
                    entry(*position, ancestor).is_some_and(|def| own_dunders(def).contains(&dunder))
                }
            })
            .map(|(_, ancestor)| ancestor)
            .expect("a candidate dunder has a user definer in the MRO");
        if is_builtin(first_definer) {
            continue;
        }
        let from = if *first_definer == class_def.name {
            String::new()
        } else {
            format!(" (defined by `{first_definer}`)")
        };
        return Err(unsupported(
            format!(
                "exception class `{}` has a user-defined `{dunder}`{from}, which pycc would \
                 not call on a raised exception value; pycc does not materialize an instance \
                 of a user-defined exception class yet (Part 3 of #541)",
                class_def.name
            ),
            range,
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "exception_dunders_tests.rs"]
mod tests;
