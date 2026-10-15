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
//! A generator of such values boxes too, as a CPython iterator over the
//! same native generator ([`is_boxable_generator`]).
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
        && (matches!(from, Ty::None)
            || foreign::is_object_operand(env, from)
            || is_boxable_generator(env, from))
}

/// Whether `ty` is a generator whose values box (`admits`): it crosses as
/// a CPython iterator sharing the native generator (`pycc_ext_gen_wrap`),
/// so, unlike a container, no copy breaks aliasing.
pub(crate) fn is_boxable_generator(env: &Environment, ty: &Ty) -> bool {
    matches!(ty, Ty::Generator(sig) if foreign::is_object_operand(env, &sig.0))
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
    // Non-strict mode (#3): a display of boxable elements becomes a fresh
    // CPython container -- no aliasing to break, since nothing else holds it.
    if *to == Ty::Object && crate::non_strict::is_non_strict() && display_boxable(value, from) {
        return Ok(true);
    }
    // Non-strict mode: so does a fresh native list nothing else holds yet.
    if *to == Ty::Object && fresh_container(env, value, from) {
        return Ok(true);
    }
    // Non-strict mode (#3): a name holding a native container that flows
    // into an `object` slot becomes an `object` itself (widening), so the
    // container it builds is the CPython one from the start.
    if let (Ty::Object, HirExpr::Name(name)) = (to, value)
        && matches!(
            from,
            Ty::List(_) | Ty::Dict(_) | Ty::Set(_) | Ty::FrozenSet(_) | Ty::Tuple(_)
        )
    {
        crate::non_strict::request_widen(name, from, &Ty::Object, |_| true);
    }
    // So does one that is an element of a tuple display flowing there
    // (`return (chars1, chars2, line_array)`): once widened, the display's
    // elements all box and it is the fresh CPython tuple above.
    if let (Ty::Object, HirExpr::TupleLiteral(elements), Ty::Tuple(element_tys)) =
        (to, value, from)
    {
        for (element, element_ty) in elements.iter().zip(element_tys.iter()) {
            if let HirExpr::Name(name) = element
                && matches!(
                    element_ty,
                    Ty::List(_) | Ty::Dict(_) | Ty::Set(_) | Ty::FrozenSet(_) | Ty::Tuple(_)
                )
            {
                crate::non_strict::request_widen(name, element_ty, &Ty::Object, |_| true);
            }
        }
    }
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

/// Non-strict mode: whether `value` is a native list of type `ty` that
/// nothing else holds yet, whose CPython copy (`pycc_ext_obj_pack_list`)
/// therefore cannot break aliasing: a display, a comprehension, a
/// `list(...)`/`sorted(...)`/`reversed(...)` result or a call of a function
/// returning a fresh list (`non_strict::fresh_returns`), a `str.split()`, a
/// `.copy()`, a concatenation, repetition or slice -- of elements that are
/// immutable or carry their identity across (a scalar, a `str`, a tuple of
/// those, a compiled instance), or, for a display or comprehension, of
/// elements that are fresh lists themselves.
pub(crate) fn fresh_container(env: &Environment, value: &HirExpr, ty: &Ty) -> bool {
    crate::non_strict::is_non_strict() && fresh_list(env, value, ty)
}

fn fresh_list(env: &Environment, value: &HirExpr, ty: &Ty) -> bool {
    let Ty::List(element) = ty else {
        return false;
    };
    let flat = shareable_element(env, element);
    match value {
        HirExpr::ListLiteral(elements) => {
            elements.iter().all(|e| flat || fresh_list(env, e, element))
        }
        HirExpr::Comprehension(comp) => {
            matches!(&comp.elt, pycc_hir::CompElt::List(e) if flat || fresh_list(env, e, element))
        }
        HirExpr::EmptyList(_) => true,
        HirExpr::Call { callee, .. } => {
            flat && if env.lookup_function(callee).is_none() {
                matches!(callee.as_str(), "list" | "sorted" | "reversed")
            } else {
                // A function every return of which is a list nothing else
                // holds (`non_strict::fresh_returns`).
                crate::non_strict::is_fresh_returning(callee)
            }
        }
        HirExpr::MethodCall { method, .. } => {
            flat && matches!(method.as_str(), "split" | "rsplit" | "splitlines" | "copy")
        }
        HirExpr::BinOp {
            op: pycc_hir::BinOpKind::Add | pycc_hir::BinOpKind::Mul,
            ..
        }
        | HirExpr::Slice { .. } => flat,
        _ => false,
    }
}

/// Whether a list element of type `ty` may be shared between a native list
/// and its CPython copy: immutable, or an instance whose carrier keeps its
/// identity.
fn shareable_element(env: &Environment, ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Float | Ty::Bool | Ty::Str => true,
        Ty::Tuple(elements) => elements
            .iter()
            .all(|e| matches!(e, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)),
        Ty::Instance(_) => foreign::is_object_operand(env, ty),
        _ => false,
    }
}

/// Whether `value` is a list, tuple, set or dict display of type `ty` whose
/// every element is a packable scalar or object, or itself such a display.
pub(crate) fn display_boxable(value: &HirExpr, ty: &Ty) -> bool {
    let element = |e: &HirExpr, t: &Ty| foreign::is_packable_operand(t) || display_boxable(e, t);
    match (value, ty) {
        (HirExpr::ListLiteral(es), Ty::List(t)) | (HirExpr::SetLiteral(es), Ty::Set(t)) => {
            es.iter().all(|e| element(e, t))
        }
        (HirExpr::TupleLiteral(es), Ty::Tuple(ts)) => {
            es.len() == ts.len() && es.iter().zip(ts.iter()).all(|(e, t)| element(e, t))
        }
        (HirExpr::DictLiteral(pairs), Ty::Dict(kv)) => pairs
            .iter()
            .all(|(k, v)| element(k, &kv.0) && element(v, &kv.1)),
        // An empty display whose element type the empty-container pass
        // resolved (`xs = []` later widened to `object`).
        (HirExpr::EmptyList(_), Ty::List(_))
        | (HirExpr::EmptySet(_), Ty::Set(_))
        | (HirExpr::EmptyDict(_), Ty::Dict(_)) => true,
        _ => false,
    }
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
