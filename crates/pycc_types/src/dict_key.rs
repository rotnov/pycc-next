//! A class instance as a dict key (`dict[Color, int]`).
//!
//! The runtime dict hashes an instance key by its address and compares two
//! by identity (`pycc_rt::value`), which is CPython's `object.__eq__`. A
//! dict's iteration order is its insertion order, so the hash itself is
//! never observable: an instance key compiles exactly when its class
//! compares by identity -- no class of its MRO binds `__eq__` (an enum
//! class, a plain class). A class with its own `__eq__` is `C0001`.

use crate::env::Environment;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{InstanceEq, Ty, resolve_instance_eq};

/// Refuses `dict_ty` when its key is an instance of a class that compares
/// by anything but identity.
pub(crate) fn check_dict_key(env: &Environment, dict_ty: &Ty) -> Result<(), Diagnostic> {
    let Ty::Dict(pair) = dict_ty else {
        return Ok(());
    };
    let Ty::Instance(class) = &pair.0 else {
        return Ok(());
    };
    if env.lookup_class(class).is_some()
        && matches!(resolve_instance_eq(class, &env.classes), InstanceEq::Identity)
        && !env.is_synthetic_class(class)
    {
        return Ok(());
    }
    Err(Diagnostic::error(
        "C0001",
        format!(
            "a dict key of class `{class}` is valid Python but not implemented yet -- only an \
             instance of a class comparing by identity (no `__eq__`, e.g. an enum member) is a \
             dict key so far"
        ),
        Span::new(0, 0),
    ))
}
