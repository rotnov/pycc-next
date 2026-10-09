//! A native value moved into an `object` slot (Part 2 of #1387, D-258's
//! boxing amendment).
//!
//! D-258 makes `object` (and, in an `--ext` module, `Any`) the opaque
//! CPython top type. `crate::is_assignable` keeps admitting `object` only
//! from `object`: it is shared by branch joins, container element checks,
//! protocol members and generic bounds, none of which has a conversion. The
//! one conversion that exists is the shim's packers (`pycc_ext_obj_pack_*`),
//! which turn a native `int`, `float`, `bool`, `str` or a compiled
//! instance's `PyccExtInstance` carrier into the `PyObject *` CPython would
//! hold, plus `pycc_ext_obj_none` for `None`.
//!
//! [`admits`] is the one statement of which values those are. Each seam
//! that moves exactly one value into exactly one slot ORs it into its own
//! assignability test:
//!
//! - a compiled function, method, constructor, static, class or `super()`
//!   call's argument (`expr.rs`, `class::check_call_args`);
//! - an annotated binding `y: object = v` and a later plain rebinding of an
//!   `object`-typed name (`crate::check_stmt*`, `crate::check_assignment`);
//! - `return v` in a function declared `-> object` (`crate::check_stmt_in_function`
//!   and the solver's `HirStmt::Return` arm);
//! - an attribute store into an `object`-typed slot, including a property
//!   setter's `object` parameter (`class::attr_set`).
//!
//! Everything else keeps the strict rule: a native container (D-258 rule 4:
//! a copy would break aliasing), an `Optional[T]` value (its `None` and its
//! payload have no single packer), an enum member and an exception
//! instance. `pycc_mir` inserts `MirExpr::ObjectBox` at the binding,
//! rebinding and attribute-store seams, and `pycc_codegen` boxes a call
//! argument and a returned value directly (its `object_box` module).
//!
//! A foreign-class annotation is also `Ty::Object` (Part 1 of #1367), so a
//! native value is admitted into one too. Annotations are not enforced at
//! runtime, so the program behaves as CPython runs it; only the static
//! strictness of a foreign-class annotation is lost.

use crate::env::Environment;
use crate::foreign;
use pycc_diag::Diagnostic;
use pycc_hir::{HirExpr, Ty};

/// Whether a value of type `from` moved into a slot of type `to` is a
/// native value boxed into an `object` slot.
///
/// `false` when `to` is not `object`, and when `from` already is one (the
/// ordinary assignability check admits that without a conversion).
pub(crate) fn admits(env: &Environment, from: &Ty, to: &Ty) -> bool {
    matches!(to, Ty::Object)
        && !matches!(from, Ty::Object)
        && (matches!(from, Ty::None) || foreign::is_object_operand(env, from))
}

/// [`admits`] for a value whose expression is known, refusing (`I0404`)
/// a class method's own `cls`, which is typed as an instance of its class
/// but holds none ([`foreign::refuse_classmethod_cls`]).
///
/// Every boxing seam with an expression in hand calls this one: an
/// annotated binding, a plain rebinding, a `return`, an attribute store and
/// every call shape's arguments (`class::check_call_args` takes the
/// argument expressions too). Only an alias (`c = cls; f(c)`) is not
/// traced; it reaches the shim's null guard and raises `SystemError`, as it
/// already does at a foreign call.
pub(crate) fn admits_value(
    env: &Environment,
    value: &HirExpr,
    from: &Ty,
    to: &Ty,
) -> Result<bool, Diagnostic> {
    if !admits(env, from, to) {
        return Ok(false);
    }
    foreign::refuse_classmethod_cls(
        env,
        value,
        "boxing a class method's `cls` into an `object` slot",
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_and_none_box_into_object() {
        let env = Environment::new();
        for from in [Ty::Int, Ty::Float, Ty::Bool, Ty::Str, Ty::None] {
            assert!(admits(&env, &from, &Ty::Object), "{from:?}");
        }
    }

    #[test]
    fn object_containers_optional_and_unknown_instances_do_not_box() {
        let env = Environment::new();
        for from in [
            Ty::Object,
            Ty::List(Box::new(Ty::Int)),
            Ty::Optional(Box::new(Ty::Int)),
            Ty::Instance(Box::new("Missing".to_string())),
        ] {
            assert!(!admits(&env, &from, &Ty::Object), "{from:?}");
        }
    }

    #[test]
    fn only_an_object_slot_boxes() {
        let env = Environment::new();
        assert!(!admits(&env, &Ty::Int, &Ty::Float));
        assert!(!admits(&env, &Ty::Str, &Ty::Str));
    }
}
