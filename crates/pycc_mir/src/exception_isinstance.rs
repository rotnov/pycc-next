//! `isinstance` on a caught builtin exception value (#1337, WI-6a).
//!
//! pycc folds `isinstance` at compile time from the value's static class
//! (#435), which D-254 makes exact for every user-class instance. A value
//! whose static type is a *builtin* exception class is different: it is the
//! runtime `PyExceptionObj` an `except` clause caught, and its dynamic class
//! can be any subclass -- `except Exception as e` after `raise KeyError`, or
//! `except ValueError as e` after `raise E` for a user `E(ValueError)`. The
//! static fold answered `False` there where CPython answers `True`. Such a
//! test now reads the object's type tag at run time, exactly as the
//! handler dispatch does.
//!
//! The fold stays for every other static type: a user exception class's
//! values are plain instances without a type tag (Part 3 of #541), and an
//! `except*` binding's `ExceptionGroup`/`BaseExceptionGroup` static type
//! keeps its fold, which already matches CPython for the groups pycc builds.

use pycc_hir::{HirClassDef, builtin_exception_class_defs, is_builtin_exception_class};
use std::collections::HashMap;

use super::exception::exception_type_tag;

/// Whether a value whose static type is `class` is a runtime
/// `PyExceptionObj` that `isinstance` must test by tag: `class` is one of
/// the seeded builtin exception classes other than the two group classes.
///
/// Seeding is all-or-nothing per program (D-188), so the program is seeded
/// exactly when every synthetic builtin class is in `classes` unchanged; a
/// program shadowing a builtin name with its own class seeds none of them,
/// and its same-named class is an ordinary one whose values carry no tag.
pub(super) fn tests_by_tag(class: &str, classes: &HashMap<String, HirClassDef>) -> bool {
    is_builtin_exception_class(class)
        && !matches!(class, "ExceptionGroup" | "BaseExceptionGroup")
        && builtin_exception_class_defs()
            .iter()
            .all(|(name, def)| classes.get(name) == Some(def))
}

/// The runtime type tags whose objects are instances of `target`: its own
/// tag when it is an exception class, and every raisable class whose MRO
/// reaches it. Empty when no raisable class is a `target` (a plain class
/// no exception class inherits), where the answer is statically `False`.
pub(super) fn isinstance_tags(target: &str, classes: &HashMap<String, HirClassDef>) -> Vec<u8> {
    let mut tags: Vec<u8> = exception_type_tag(target, classes).into_iter().collect();
    tags.extend(classes.values().filter_map(|def| {
        def.exception_type_tag
            .filter(|_| def.name != target && def.mro.iter().any(|ancestor| ancestor == target))
    }));
    tags
}

#[cfg(test)]
#[path = "exception_isinstance_tests.rs"]
mod tests;
