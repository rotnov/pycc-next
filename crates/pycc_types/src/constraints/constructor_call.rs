//! The constraint solver's term for a user-class constructor call (#1342):
//! `C(args)` on a non-generic class `C` of the module's own class table
//! answers `Ty::Instance(C)`, so an unannotated private helper whose body is
//! `return C(...)` (directly, or through a local bound from one) infers its
//! return as `C` instead of reporting `T0021`.
//!
//! **Placement.** The `Call` arm consults this only after every other
//! reading of the callee has missed: a value binding, a foreign import, a
//! local, the hand-recognized builtins, a function signature, and
//! `is_known_callable_builtin`. A class named like a builtin
//! (`class range:`, a user `class frozenset:`) and the builtin exception
//! classes HIR lowering seeds into the same class table (`ValueError`, ...)
//! therefore keep their earlier diagnostics byte for byte: the solver's
//! environment carries no provenance flag that would tell a seeded class
//! from a user-authored one, and this placement makes it unnecessary. Only
//! the former `Ok(None)` -- an unresolved callee, which left the helper's
//! return to signature materialization's `T0021` -- changes.
//!
//! **What it does not check.** The check phase stays the authority on
//! whether the instantiation is valid (`class::resolve_instantiation`): an
//! abstract class still answers its instance here, so the check phase's
//! precise `C0001` is what the program is refused with, rather than a
//! misleading "cannot infer return type". A protocol class answers nothing:
//! with `_mk` inferred as `P`, the check phase's own `resolve_method_call`
//! would refuse a top-level `_mk().f()` with `T0044` (a protocol's stub
//! methods are `protocol_members`, not callable `methods`), and that
//! pass-2 error is reported alone, masking the pass-3 "cannot instantiate
//! protocol class" of `_mk`'s body (`crate::module`'s pass ordering). So
//! the helper keeps its prior `T0021`. An enum call is refused by HIR
//! lowering itself (`pycc_hir::enum_class_call_message`) in all but the
//! order-dependent shapes `class::binding` documents; there the check
//! phase's `C0001` stays the refusal, though a top-level use of the
//! helper's result may report its own error first. The guard is
//! deliberately protocol-only: the protocol case is the one pinned shape
//! where the instance answer replaced a refusal with a misleading one. The arguments
//! are collected by the `Call` arm for their own constraints but
//! deliberately not unified with `__init__`'s parameters, on
//! `method_return`'s reasoning for a method call: a method parameter is never
//! inferred from its call sites (`docs/TYPE_SYSTEM.md`, "v0.1 local
//! inference"). So an unannotated helper parameter passed only to a
//! constructor (`def _mk(v): return C(v)`) stays uninferred.
//!
//! A generic class (`class C[T]:`) answers nothing: a bare `C(x)` has no
//! type argument to instantiate with, and this solver has no
//! generic-instantiation step; `C[int](x)` is its own HIR shape
//! (`GenericClassInstantiate`) and never reaches the `Call` arm.

use super::*;

/// `Ty::Instance(callee)` when `callee` names a non-generic, non-protocol
/// class in the module's class table; `None` otherwise (the `Call` arm
/// then keeps its historical "no term").
pub(super) fn constructor_call_term(
    env: &ConstraintEnvironment<'_, '_>,
    callee: &str,
) -> Option<TypeTerm> {
    let (name, def) = env.class_defs.iter().find(|(name, _)| name == callee)?;
    if def.type_param.is_some() || def.is_protocol {
        return None;
    }
    Some(Ok(Ty::Instance(Box::new(name.clone()))))
}
