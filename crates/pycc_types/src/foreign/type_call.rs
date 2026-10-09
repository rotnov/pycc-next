//! `type(o)` on a CPython object (Part 11 of #1371).
//!
//! The one-argument `type` builtin applied to an `object` value is
//! CPython's own `PyObject_Type`, whose result is the operand's class: one
//! more opaque object. Every other `type(...)` shape keeps the
//! known-builtin `C0001` it had: a native argument, the three-argument
//! class constructor and any arity other than one. Calling the result
//! (`type(o)(...)`) stays refused in HIR, which reads only
//! `type(self)(...)` as a call on a call (#1411).
//!
//! Both callers -- `crate::expr`'s checker arm and `crate::constraints`'
//! solver arm -- sit after the class, generic and user-function lookups, so
//! a program's own `def type` or `class type` keeps its meaning, and both
//! decline for a stdlib module alias spelled `type` (`import math as type`),
//! which lives in none of those tables. `pycc_mir` mirrors the same
//! precedence: its class-instantiation lookup runs first, and its own split
//! carries the `$fn:type` shadow guard.
//!
//! The solver arm also reads an argument whose term is still unresolved
//! (an unannotated parameter) as an object; the final check pass re-types
//! the call with the resolved argument, so a native one keeps its `C0001`.
//! Both callers also pass a bare read of an `object` name an `isinstance`
//! guard narrowed as the object itself (#1476), so `type(o)` reports the
//! object's own class, an `int` subclass's included.

use pycc_hir::Ty;

/// The builtin's spelling.
pub(crate) const TYPE: &str = "type";

/// Whether `callee(args)` is `type(o)` on one CPython object argument,
/// typed as an `object`. The caller has already consulted the class,
/// generic and user-function tables.
///
/// `std_module_aliases` is the scope's stdlib module aliases, which both
/// `Environment` and the solver's `ConstraintEnvironment` carry.
pub(crate) fn is_object_type_call(
    std_module_aliases: &[(String, pycc_std::StdModule)],
    callee: &str,
    arg_tys: &[Ty],
) -> bool {
    callee == TYPE
        && matches!(arg_tys, [Ty::Object])
        && !std_module_aliases.iter().any(|(alias, _)| alias == callee)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_object_argument_is_admitted() {
        assert!(is_object_type_call(&[], "type", &[Ty::Object]));
        assert!(!is_object_type_call(&[], "type", &[Ty::Int]));
        assert!(!is_object_type_call(&[], "type", &[]));
        assert!(!is_object_type_call(&[], "type", &[Ty::Object, Ty::Object]));
        assert!(!is_object_type_call(&[], "len", &[Ty::Object]));
    }

    #[test]
    fn a_stdlib_module_alias_spelled_type_declines() {
        let aliases = [("type".to_string(), pycc_std::StdModule::Math)];
        assert!(!is_object_type_call(&aliases, "type", &[Ty::Object]));
    }
}
