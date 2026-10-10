//! Typing of the `list[T]` operations beyond indexing, slicing, `.append()`
//! and `.pop()`: concatenation and repetition, membership, item assignment,
//! `.insert(i, v)` / `.pop(i)`, and the self-rebinding refusal that keeps an
//! augmented assignment's in-place semantics from being miscompiled.
//!
//! Every element type `pycc_hir::is_list_element_ty` admits is handled here
//! generically: the runtime stores each element as one native word, so no
//! rule below depends on which element type a list holds, except membership
//! of a user-class instance, which is decided by identity and so is admitted
//! only for a class without `__eq__`.

use crate::{Environment, is_assignable};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{BinOpKind, HirExpr, Ty};

fn mismatch(message: String) -> Diagnostic {
    Diagnostic::error("T0021", message, Span::new(0, 0))
}

/// `list + list` and `list * int` / `int * list`; `None` when neither
/// operand is a list, so the caller's numeric rules apply unchanged.
pub(crate) fn list_binop_ty(
    op: BinOpKind,
    left: &Ty,
    right: &Ty,
) -> Option<Result<Ty, Diagnostic>> {
    let is_count = |t: &Ty| matches!(t, Ty::Int | Ty::Bool);
    match (op, left, right) {
        (BinOpKind::Mul, Ty::List(_), count) if is_count(count) => Some(Ok(left.clone())),
        (BinOpKind::Mul, count, Ty::List(_)) if is_count(count) => Some(Ok(right.clone())),
        (BinOpKind::Add, Ty::List(_), Ty::List(_)) if left == right => Some(Ok(left.clone())),
        (_, Ty::List(_), _) | (_, _, Ty::List(_)) => Some(Err(mismatch(format!(
            "operator {op:?} is not defined for `{}` and `{}`",
            left.name(),
            right.name()
        )))),
        _ => None,
    }
}

/// Whether `class` (or a class in its MRO) defines `__eq__`, a dataclass's
/// synthesized one included.
fn class_has_eq(env: &Environment, class: &str) -> bool {
    let Some(def) = env.lookup_class(class) else {
        return false;
    };
    def.mro.iter().any(|name| {
        env.lookup_class(name)
            .is_some_and(|c| c.is_dataclass || c.methods.iter().any(|(m, _)| m == "__eq__"))
    })
}

/// `item in xs` / `item not in xs` over a `list[T]`: `item` must be
/// assignable to `T`. An instance element compares by identity, which is
/// CPython's answer only when the class has no `__eq__`.
pub(crate) fn membership_ty(env: &Environment, item: &Ty, elem: &Ty) -> Result<Ty, Diagnostic> {
    if !is_assignable(item.clone(), elem.clone()) {
        return Err(mismatch(format!(
            "cannot test `{}` for membership in a `list[{}]`",
            item.name(),
            elem.name()
        )));
    }
    if let Ty::Instance(class) = elem
        && class_has_eq(env, class)
    {
        return Err(Diagnostic::error(
            "C0001",
            format!("`in` over a list of `{class}`, which defines `__eq__`, is not supported yet"),
            Span::new(0, 0),
        ));
    }
    Ok(Ty::Bool)
}

fn require_index(index: &Ty, what: &str) -> Result<(), Diagnostic> {
    if is_assignable(index.clone(), Ty::Int) {
        return Ok(());
    }
    Err(
        mismatch(format!("{what} must be `int`, found `{}`", index.name()))
            .with_help("use an `int` value"),
    )
}

fn require_element(value: &Ty, elem: &Ty, what: &str) -> Result<(), Diagnostic> {
    if is_assignable(value.clone(), elem.clone()) {
        return Ok(());
    }
    Err(mismatch(format!(
        "cannot {what} `{}` to a list of `{}`",
        value.name(),
        elem.name()
    )))
}

/// Refuses rendering (`print`, an f-string, `str()`) a list whose elements
/// are, at any nesting depth, user-class instances: the runtime renders
/// every other element kind exactly as CPython does, but an instance's
/// `repr()` is its class's `__repr__`, which it cannot call.
pub(crate) fn check_list_renderable(ty: &Ty) -> Result<(), Diagnostic> {
    let mut element = ty;
    while let Ty::List(inner) = element {
        element = inner;
    }
    if let Ty::Instance(class) = element {
        return Err(Diagnostic::error(
            "C0001",
            format!(
                "rendering `{}` (a list of `{class}` instances) as a string is not supported yet",
                ty.name()
            ),
            Span::new(0, 0),
        ));
    }
    Ok(())
}

/// `xs[index] = value` on a `list[T]`.
pub(crate) fn check_list_set(elem: &Ty, index: &Ty, value: &Ty) -> Result<(), Diagnostic> {
    require_index(index, "list index")?;
    require_element(value, elem, "assign")
}

/// A method call on a `list[T]` receiver that is not one of the
/// hand-recognized `.append(v)` / `.pop()` nodes: `.insert(i, v)` and
/// `.pop(i)`. Every other method is refused.
pub(crate) fn list_method_ty(elem: &Ty, method: &str, arg_tys: &[Ty]) -> Result<Ty, Diagnostic> {
    match (method, arg_tys) {
        ("insert", [index, value]) => {
            require_index(index, "list.insert() index")?;
            require_element(value, elem, "insert")?;
            Ok(Ty::None)
        }
        ("pop", [index]) => {
            require_index(index, "list.pop() index")?;
            Ok(elem.clone())
        }
        _ => Err(Diagnostic::error(
            "C0001",
            format!(
                "`list.{method}()` with {} argument(s) is not supported yet",
                arg_tys.len()
            ),
            Span::new(0, 0),
        )),
    }
}

/// Refuses `target = target + other` and `target = target * n` for a list
/// `target` (and so `target += other`, which `pycc_hir` rewrites to exactly
/// that shape before any type exists). CPython's `+=`/`*=` mutate the list
/// in place, which every other reference to it observes; the rewritten form
/// would build a new list instead. Both spellings are refused, because the
/// two are indistinguishable here.
pub(crate) fn refuse_list_self_rebind(
    target: &HirExpr,
    value: &HirExpr,
    value_ty: &Ty,
) -> Result<(), Diagnostic> {
    if let (
        Ty::List(_),
        HirExpr::BinOp {
            op: BinOpKind::Add | BinOpKind::Mul,
            left,
            ..
        },
    ) = (value_ty, value)
        && left.as_ref() == target
    {
        return Err(Diagnostic::error(
            "C0001",
            "rebinding a list to `+`/`*` of itself (or `+=`/`*=` on a list) is not supported yet \
             -- CPython's `+=` mutates the list in place"
                .to_string(),
            Span::new(0, 0),
        ));
    }
    Ok(())
}
