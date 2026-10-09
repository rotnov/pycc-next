//! Compile-time `isinstance`/`issubclass` evaluation and the class-name
//! predicates it is built on (issue #435).
//!
//! Extracted from `lib.rs` per AGENTS.md's file-decomposition rule (issue
//! #547, Part 2). Unlike this crate's other submodules, everything here is
//! public API rather than crate-internal, so `lib.rs` re-exports all eight
//! items and the `pycc_hir::is_builtin_type_name`-style paths are unchanged
//! by the move. Counting non-comment references outside `lib.rs`:
//! `is_builtin_type_name` 7 in `tests.rs`, 2 in `pycc_mir`, 2 in
//! `pycc_types`; `eval_isinstance_single` 15 and 2; `eval_issubclass_single`
//! 12 and 2; `extract_class_names` 6, 3, and 3; the three base-name
//! predicates 1-2 each, all from this crate's `class.rs`.

use crate::{HirExpr, Ty};

// ---------------------------------------------------------------------------
// Issue #435: compile-time `isinstance`/`issubclass` evaluation helpers.
//
// pycc uses static dispatch (D-006) — `is_assignable` does not allow
// `Ty::Instance("D")` to be assigned to `Ty::Instance("B")`, so every
// variable's runtime type is exactly its declared static type. Therefore
// `isinstance` and `issubclass` can always be evaluated at compile time,
// emitting constant boolean values, with one exception (#1337): a value typed
// as a seeded builtin exception class is a caught runtime exception object,
// and `pycc_mir` tests a target outside its static MRO by the object's
// existing type tag (`MirExpr::ExceptionTypeTest`).
//
// These helpers are shared by `pycc_types` (type checker) and `pycc_mir`
// (MIR lowering) so both compute identical results from the same inputs.
// ---------------------------------------------------------------------------

/// Returns `true` if `name` is one of the builtin scalar type names pycc
/// recognizes as a valid class argument to `isinstance`/`issubclass`:
/// `int`, `str`, `float`, `bool`.
pub fn is_builtin_type_name(name: &str) -> bool {
    matches!(name, "int" | "str" | "float" | "bool")
}

/// Returns `true` if `name` is a builtin enum base name recognized by
/// `lower_class` as a marker that a class is a PEP 435 enum: `Enum`
/// (#379, PR-19) or `StrEnum` (#892). Neither is a user-defined class in
/// `class_defs` -- both are builtin base names consumed as markers, not
/// recorded in the class's `bases`/`mro`. `pycc_std` registers `enum.Enum`
/// and `enum.StrEnum` as `EnumMarker` symbols so `from enum import Enum`
/// and `from enum import StrEnum` resolve (the import is a no-op binding —
/// neither is ever a first-class value, only a base class marker). The bare
/// names (without any import) are also accepted, matching pycc's existing
/// textual-resolution precedent for `math.sqrt`. `StrEnum` additionally
/// fixes the class's member-value type to `str`; see `lower_enum_class`.
pub fn is_enum_base_name(name: &str) -> bool {
    name == "Enum" || name == "StrEnum"
}

/// Returns `true` if `name` is the builtin `Protocol` base name recognized
/// by `lower_class` as a marker that a class is a PEP 544 protocol (#380,
/// PR-20). `Protocol` is not a user-defined class in `class_defs` -- it is
/// a builtin base name consumed as a marker, not recorded in the class's
/// `bases`/`mro`. `pycc_std` registers `typing.Protocol` as a
/// `ProtocolMarker` symbol so `from typing import Protocol` resolves (the
/// import is a no-op binding — `Protocol` is never a first-class value,
/// only a base class marker). The bare name `Protocol` (without any
/// import) is also accepted, matching pycc's existing textual-resolution
/// precedent for `Enum`.
pub fn is_protocol_base_name(name: &str) -> bool {
    name == "Protocol"
}

/// Returns `true` if `name` is the builtin `ABC` base name recognized by
/// `lower_class` as a marker that a class is abstract (PEP 3119, #380,
/// PR-20). `ABC` is not a user-defined class in `class_defs` -- it is a
/// builtin base name consumed as a marker, not recorded in the class's
/// `bases`/`mro`. `pycc_std` registers `abc.ABC` as an `AbcMarker` symbol
/// so `from abc import ABC` resolves (the import is a no-op binding —
/// `ABC` is never a first-class value, only a base class marker). The bare
/// name `ABC` (without any import) is also accepted, matching pycc's
/// existing textual-resolution precedent for `Enum`/`Protocol`.
pub fn is_abc_base_name(name: &str) -> bool {
    name == "ABC"
}

/// Computes the compile-time result of `isinstance(obj, target_class)`.
///
/// `obj_ty` is the inferred static type of the object expression.
/// `target_class` is the class name from the second argument (already
/// validated as either a user-defined class or a builtin type name).
/// `obj_mro` is the MRO of the object's class (if `obj_ty` is
/// `Ty::Instance`); for non-instance types it is unused.
///
/// Builtin subtype rules: `bool` is a subtype of `int` (matching CPython's
/// own type hierarchy where `bool` inherits from `int`).
pub fn eval_isinstance_single(obj_ty: &Ty, target_class: &str, obj_mro: &[String]) -> bool {
    match obj_ty {
        Ty::Instance(_) => obj_mro.iter().any(|c| c == target_class),
        Ty::Int => target_class == "int",
        Ty::Bool => target_class == "bool" || target_class == "int",
        Ty::Str => target_class == "str",
        Ty::Float => target_class == "float",
        _ => false,
    }
}

/// Computes the compile-time result of `issubclass(cls, target_class)`.
///
/// `cls` is the source class name from the first argument (already
/// validated as either a user-defined class or a builtin type name).
/// `target_class` is the class name from the second argument.
/// `cls_mro` is the MRO of the source class (if it is a user-defined
/// class); for builtin types it is unused.
///
/// Builtin subtype rules: `issubclass(bool, int)` is `true` (matching
/// CPython's own type hierarchy). Same-builtin comparisons (`issubclass(int,
/// int)`) are `true`.
pub fn eval_issubclass_single(cls: &str, target_class: &str, cls_mro: &[String]) -> bool {
    if is_builtin_type_name(cls) {
        if cls == "bool" && target_class == "int" {
            return true;
        }
        return cls == target_class;
    }
    // User class: check if target is in the source class's MRO.
    // The MRO includes the class itself, so `issubclass(D, D)` is true.
    if is_builtin_type_name(target_class) {
        // A user class is not a subclass of a builtin type (pycc's MRO
        // does not include `object` or any builtin).
        return false;
    }
    cls_mro.iter().any(|c| c == target_class)
}

/// Extracts class names from an `isinstance`/`issubclass` class argument
/// expression. The argument must be either:
/// - `HirExpr::Name(name)` — a single class name
/// - `HirExpr::TupleLiteral(elements)` — a tuple of class names, where each
///   element is `HirExpr::Name(name)`
///
/// Returns `Ok(names)` if the expression matches one of these shapes,
/// or `Err(ExtractClassNamesError)` if it doesn't (the caller produces the
/// appropriate diagnostic). An empty tuple is rejected (at least one class
/// is required).
pub fn extract_class_names(arg: &HirExpr) -> Result<Vec<String>, ExtractClassNamesError> {
    match arg {
        HirExpr::Name(name) => Ok(vec![name.clone()]),
        HirExpr::TupleLiteral(elements) => {
            if elements.is_empty() {
                return Err(ExtractClassNamesError);
            }
            let mut names = Vec::with_capacity(elements.len());
            for elem in elements {
                match elem {
                    HirExpr::Name(name) => names.push(name.clone()),
                    _ => return Err(ExtractClassNamesError),
                }
            }
            Ok(names)
        }
        _ => Err(ExtractClassNamesError),
    }
}

/// Error returned by [`extract_class_names`] when the argument is not a
/// valid class name or tuple of class names. The caller is responsible for
/// producing the appropriate diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractClassNamesError;
