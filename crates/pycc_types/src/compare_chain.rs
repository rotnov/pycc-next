//! Comparison typing shared by a single comparison and a chained
//! comparison `a < b < c` (#1212, Part 4 of #1018).
//!
//! A chain admits exactly what a single comparison admits, link by link:
//! each adjacent operand pair is typed by [`compare_link_ty`] exactly as
//! `a op b` would be, and the chain's result is `bool`.

use crate::Environment;
use crate::expr::infer_expr_in;
use crate::numeric_or_bool_compatible;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{CmpOpKind as CmpOp, CompareLink, HirExpr, Ty};

/// Types one comparison link `left op right` whose operand types are
/// already inferred. The `is`/`is not` arm needs both operand expressions,
/// not only their types, because it keys on whether either side is the
/// `None` literal.
pub(crate) fn compare_link_ty(
    env: &Environment,
    op: CmpOp,
    left: &HirExpr,
    right: &HirExpr,
    left_ty: &Ty,
    right_ty: &Ty,
) -> Result<Ty, Diagnostic> {
    // `is`/`is not` (D-197, #763, Part 1 of #747): with a `None` literal
    // operand, the *other* operand's static type must be `Ty::Optional(_)`,
    // `Ty::None` itself, or (Part 1 of #1371) a CPython object, whose
    // `None` test is pointer identity against `Py_None`. Without a `None`
    // literal operand, HIR lowering (`pycc_hir::compare_chain::lower_cmp_op`)
    // admits a single comparison or (Part 11 of #1371) a chain link
    // between two non-literal operands, and
    // `foreign::compare::general_identity_ty` admits only two objects.
    // `opt == None` of an `Optional[T]`: `T`'s `__eq__` answers
    // `NotImplemented` for `None`, so it is the identity test (`pycc_mir`).
    if matches!(op, CmpOp::Eq | CmpOp::NotEq)
        && (matches!(left, HirExpr::NoneLiteral) && matches!(right_ty, Ty::Optional(_))
            || matches!(right, HirExpr::NoneLiteral) && matches!(left_ty, Ty::Optional(_)))
    {
        return Ok(Ty::Bool);
    }
    // `opt == v` / `opt != v` of an `Optional[T]` and a `T` (or two
    // `Optional[T]`s): equal when both are `None`, or both hold values that
    // compare equal (`pycc_codegen::optional_compare`). A class instance
    // `T` is left out: its own `__eq__` would have to see the `None`.
    if matches!(op, CmpOp::Eq | CmpOp::NotEq) && optional_equality(left_ty, right_ty) {
        return Ok(Ty::Bool);
    }
    if matches!(op, CmpOp::Is | CmpOp::IsNot) {
        // #1476: identity is the object's own, so a narrowed `object` name
        // is typed as the object it still is, never its narrowed native
        // copy (`pycc_mir` reads it raw the same way).
        let raw_ty = |operand: &HirExpr, ty: &Ty| {
            if crate::narrow::is_bare_narrowed_object_read(env, operand) {
                Ty::Object
            } else {
                ty.clone()
            }
        };
        let (left_ty, right_ty) = (&raw_ty(left, left_ty), &raw_ty(right, right_ty));
        // `x is True` / `x is False`: identity with a `bool` singleton --
        // equality for a `bool`, CPython's own identity for an object, and
        // never the same object for any other native value (`pycc_mir`).
        if matches!(left, HirExpr::BoolLiteral(_)) || matches!(right, HirExpr::BoolLiteral(_)) {
            let other_ty = if matches!(left, HirExpr::BoolLiteral(_)) {
                right_ty
            } else {
                left_ty
            };
            return match other_ty {
                Ty::Bool | Ty::Object | Ty::Int | Ty::Float | Ty::Str => Ok(Ty::Bool),
                other => Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "cannot compare `{}` and a `bool` literal with `is`/`is not`",
                        other.name()
                    ),
                    Span::new(0, 0),
                )),
            };
        }
        let other_ty = if matches!(left, HirExpr::NoneLiteral) {
            right_ty
        } else if matches!(right, HirExpr::NoneLiteral) {
            left_ty
        } else if matches!(
            (left_ty, right_ty),
            (Ty::Instance(_), Ty::Instance(_))
        ) {
            // Two class instances: identity, a pointer comparison.
            return Ok(Ty::Bool);
        } else {
            return crate::foreign::compare::general_identity_ty(op, left_ty, right_ty);
        };
        return match other_ty {
            Ty::Optional(_) | Ty::None | Ty::Object => Ok(Ty::Bool),
            // Non-strict mode: a native scalar is never `None`.
            Ty::Int | Ty::Float | Ty::Bool | Ty::Str if crate::non_strict::is_non_strict() => {
                Ok(Ty::Bool)
            }
            other => Err({
                // A name tested against `None` that is bound to `None`
                // elsewhere (`while cur is not None:` ... `cur = None`) is
                // `T | None` (`crate::num_widen` retries the check).
                let operand = if matches!(left, HirExpr::NoneLiteral) { right } else { left };
                if let HirExpr::Name(name) = operand {
                    crate::num_widen::request(name, &Ty::None, other);
                }
                Diagnostic::error(
                "T0021",
                format!(
                    "cannot compare `{}` and `None` with `is`/`is not` -- only an `Optional[T]` (or `None`) operand is supported",
                    other.name()
                ),
                Span::new(0, 0),
            )}),
        };
    }
    // Part 2b of #1371: membership answers for every pair, so a native
    // pair never falls through to the numeric rule below.
    if matches!(op, CmpOp::In | CmpOp::NotIn) {
        // An `IntEnum` needle in a container of `int`s is its value
        // (`pycc_hir::int_enum`).
        let read_int;
        let left_ty = if crate::int_enum_needle(right_ty) {
            read_int = pycc_hir::int_enum::as_int(left_ty);
            &read_int
        } else {
            left_ty
        };
        // Substring membership (`pycc_rt_str_contains`).
        if (left_ty, right_ty) == (&Ty::Str, &Ty::Str) {
            return Ok(Ty::Bool);
        }
        // A CPython object needle keeps the foreign refusal below.
        if let Ty::List(elem) | Ty::VarTuple(elem) = right_ty
            && *left_ty != Ty::Object
        {
            // Numbers of different kinds compare by value (`1.0 in [1, 2]`,
            // `pycc_rt_list_contains_num`).
            let number = |ty: &Ty| matches!(ty, Ty::Int | Ty::Bool | Ty::Float | Ty::Num);
            if number(left_ty) && number(elem) {
                return Ok(Ty::Bool);
            }
            return crate::list_ops::membership_ty(env, left_ty, elem);
        }
        // A dict key or a hashable set element: the runtime's hash lookup.
        if let Ty::Dict(kv) = right_ty
            && *left_ty != Ty::Object
        {
            return crate::list_ops::membership_ty(env, left_ty, &kv.0);
        }
        if let Ty::Set(elem) | Ty::FrozenSet(elem) = right_ty
            && *left_ty != Ty::Object
            && pycc_hir::is_hashable_ty(elem)
        {
            return crate::list_ops::membership_ty(env, left_ty, elem);
        }
        // An instance in a set of instances of its class: the set's own
        // hash and `__eq__` probe (`pycc_mir::expr::instance_order`).
        if let Ty::Set(elem) | Ty::FrozenSet(elem) = right_ty
            && matches!(**elem, Ty::Instance(_))
            && left_ty == &**elem
        {
            crate::set_element::check_set_element(elem, env)?;
            return Ok(Ty::Bool);
        }
        return crate::foreign::compare::membership_ty(op, left_ty, right_ty);
    }
    // Part 1 of #1371: a rich comparison with a CPython object operand is
    // CPython's own `PyObject_RichCompare`, whose result is an object.
    if let Some(result) =
        crate::foreign::compare::rich_compare_ty(env, (left, left_ty), (right, right_ty))
    {
        return result;
    }
    // A comparison between two instances of one class: the operator's
    // dunder (`__eq__`, `__lt__`, ...; `!=` falls back to `__eq__`),
    // which must take the other instance and return `bool`; `==`/`!=`
    // without `__eq__` is identity. `pycc_mir::compare_chain` resolves
    // the same MRO walk. Different-class comparisons stay T0021.
    if let (Ty::Instance(left_class), Ty::Instance(right_class)) = (left_ty, right_ty)
        && left_class == right_class
        && env.lookup_class(left_class).is_some()
        && !env.is_synthetic_class(left_class)
        && let Some(result) = instance_compare_ty(env, op, left_ty, right, right_ty)
    {
        return result;
    }
    if let Some(result) = container_compare_ty(op, left_ty, right_ty) {
        return result;
    }
    // An `IntEnum` member compares with a number as its `int` value
    // (`pycc_hir::int_enum`).
    if numeric_or_bool_compatible(
        pycc_hir::int_enum::as_int(left_ty),
        pycc_hir::int_enum::as_int(right_ty),
    ) {
        Ok(Ty::Bool)
    } else {
        Err(Diagnostic::error(
            "T0021",
            format!(
                "cannot compare `{}` and `{}`",
                left_ty.name(),
                right_ty.name()
            ),
            Span::new(0, 0),
        ))
    }
}

/// Types a [`HirExpr::CompareChain`]: every operand is inferred once, left
/// to right, then each adjacent pair is typed by [`compare_link_ty`]. The
/// first failing link's diagnostic wins, and the result is `bool`.
pub(crate) fn infer_compare_chain(
    env: &Environment,
    local_names: &[&str],
    first: &HirExpr,
    links: &[CompareLink],
) -> Result<Ty, Diagnostic> {
    let mut operands = Vec::with_capacity(links.len() + 1);
    operands.push((first, infer_expr_in(env, local_names, first)?));
    for link in links {
        operands.push((&link.right, infer_expr_in(env, local_names, &link.right)?));
    }
    // #1476: an `is`/`is not` link compares the object itself, which a
    // chain cannot carry, so a bare narrowed read on either side of such a
    // link is the object here and refused like any object operand; a
    // narrowed read in an ordered link (`0 < o < 10`) stays native.
    let operand_tys: Vec<Ty> = operands
        .iter()
        .enumerate()
        .map(|(index, (operand, ty))| {
            let in_identity_link = [index.checked_sub(1), Some(index)]
                .into_iter()
                .flatten()
                .filter_map(|link| links.get(link))
                .any(|link| matches!(link.op, CmpOp::Is | CmpOp::IsNot));
            if in_identity_link && crate::narrow::is_bare_narrowed_object_read(env, operand) {
                Ty::Object
            } else {
                ty.clone()
            }
        })
        .collect();
    // A chain over a CPython object is the conjunction of its links, each
    // an ordinary object comparison (`pycc_hir::chain_as_conjunction`).
    if operand_tys.contains(&Ty::Object)
        && let Some(conjunction) = pycc_hir::chain_as_conjunction(first, links)
    {
        return infer_expr_in(env, local_names, &conjunction);
    }
    crate::foreign::compare::reject_object_in_chain(&operand_tys)?;
    for (index, link) in links.iter().enumerate() {
        let (left, left_ty) = &operands[index];
        let (right, right_ty) = &operands[index + 1];
        compare_link_ty(env, link.op, left, right, left_ty, right_ty)?;
    }
    Ok(Ty::Bool)
}

/// `==`/`!=` between two containers of one type, and `<`/`<=`/`>`/`>=`
/// between two lists or tuples of one type whose elements are ordered
/// (`pycc_rt::value` compares them element by element, as CPython does).
/// `None` when the operands are not two containers.
/// Types `left op right` between two instances of one class (see
/// [`compare_link_ty`]): `None` when the class defines no dunder for an
/// ordering operator, which then stays `T0021`.
fn instance_compare_ty(
    env: &Environment,
    op: CmpOp,
    left_ty: &Ty,
    right: &HirExpr,
    right_ty: &Ty,
) -> Option<Result<Ty, Diagnostic>> {
    let Ty::Instance(class_name) = left_ty else {
        return None;
    };
    let candidates: &[&str] = match op {
        CmpOp::Eq => &["__eq__"],
        CmpOp::NotEq => &["__ne__", "__eq__"],
        CmpOp::Lt => &["__lt__"],
        CmpOp::LtE => &["__le__"],
        CmpOp::Gt => &["__gt__"],
        CmpOp::GtE => &["__ge__"],
        _ => return None,
    };
    let class_def = env.lookup_class(class_name)?;
    let declared = |method: &str| {
        class_def.mro.iter().any(|mro_class| {
            env.lookup_class(mro_class)
                .is_some_and(|def| def.methods.iter().any(|(name, _)| name == method))
        })
    };
    let Some(method) = candidates.iter().copied().find(|m| declared(m)) else {
        // Identity, as `object.__eq__` compares.
        return matches!(op, CmpOp::Eq | CmpOp::NotEq).then_some(Ok(Ty::Bool));
    };
    let result = crate::class::resolve_method_call(
        env,
        left_ty,
        method,
        std::slice::from_ref(right),
        std::slice::from_ref(right_ty),
    );
    Some(match result {
        Ok(Ty::Bool) => Ok(Ty::Bool),
        Ok(other) => Err(Diagnostic::error(
            "T0021",
            format!(
                "`{class_name}.{method}` returns `{}`; a comparison dunder pycc calls must \
                 return `bool`",
                other.name()
            ),
            Span::new(0, 0),
        )),
        Err(err) => Err(err),
    })
}

fn container_compare_ty(op: CmpOp, left: &Ty, right: &Ty) -> Option<Result<Ty, Diagnostic>> {
    let container = |ty: &Ty| {
        matches!(
            ty,
            Ty::List(_)
                | Ty::Tuple(_)
                | Ty::ShapedList(_)
                | Ty::VarTuple(_)
                | Ty::Dict(_)
                | Ty::Set(_)
                | Ty::FrozenSet(_)
        )
    };
    if !container(left) || !container(right) {
        return None;
    }
    // A fixed-arity tuple compares with a `tuple[T, ...]` as the tuple
    // it is.
    let fixed_as_var = |fixed: &Ty, var: &Ty| match (fixed, var) {
        (Ty::Tuple(items), Ty::VarTuple(element)) => pycc_hir::tuple_fits_var_tuple(items, element),
        _ => false,
    };
    let (left, right) = if fixed_as_var(left, right) {
        (right, right)
    } else if fixed_as_var(right, left) {
        (left, left)
    } else {
        (left, right)
    };
    let refuse = || {
        Err(Diagnostic::error(
            "T0021",
            format!("cannot compare `{}` and `{}`", left.name(), right.name()),
            Span::new(0, 0),
        ))
    };
    if left != right && !numerically_alike(left, right) {
        return Some(refuse());
    }
    // The runtime compares elements by value (`pycc_rt::value`): a
    // user-class instance element needs its own `__eq__`/`__hash__`, and an
    // instance set is a different runtime object (`int_set.rs`) that the
    // value routines must never read as a hash table.
    if !pycc_hir::builtins::value_comparable(left) || !pycc_hir::builtins::value_comparable(right) {
        return Some(Err(Diagnostic::error(
            "T0021",
            format!(
                "cannot compare `{}` and `{}` -- comparing containers of class instances is not compiled yet",
                left.name(),
                right.name()
            ),
            Span::new(0, 0),
        )));
    }
    if matches!(op, CmpOp::Eq | CmpOp::NotEq) {
        return Some(Ok(Ty::Bool));
    }
    // Set ordering is inclusion (`a <= b` is `a.issubset(b)`).
    let inclusion = matches!(left, Ty::Set(e) | Ty::FrozenSet(e) if pycc_hir::is_hashable_ty(e));
    Some(if (ordered(left) && ordered(right)) || inclusion { Ok(Ty::Bool) } else { refuse() })
}

/// Whether `<` is defined between two values of `ty`.
/// Whether two different sequence types compare as CPython compares them
/// with no `TypeError`: lists (or tuples) whose elements are pairwise of
/// one type or both numbers (`[1, 2] == [1.0, 2.0]`), tuples of any
/// arities so (`(1, 2.5) < (1.0, 3, 0)`; the runtime compares items by
/// their own kinds, the shorter tuple first when a prefix is equal).
fn numerically_alike(left: &Ty, right: &Ty) -> bool {
    let number = |ty: &Ty| matches!(ty, Ty::Int | Ty::Bool | Ty::Float | Ty::Num);
    let item = |a: &Ty, b: &Ty| a == b || (number(a) && number(b)) || numerically_alike(a, b);
    match (left, right) {
        (Ty::List(a), Ty::List(b)) | (Ty::VarTuple(a), Ty::VarTuple(b)) => item(a, b),
        (Ty::Tuple(xs), Ty::Tuple(ys)) | (Ty::ShapedList(xs), Ty::ShapedList(ys)) => {
            xs.iter().zip(ys.iter()).all(|(a, b)| item(a, b))
        }
        (Ty::Tuple(xs), Ty::VarTuple(e)) | (Ty::VarTuple(e), Ty::Tuple(xs)) => {
            xs.iter().all(|a| item(a, e))
        }
        _ => false,
    }
}

fn ordered(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Bool | Ty::Float | Ty::Num | Ty::Str => true,
        Ty::List(elem) | Ty::VarTuple(elem) => ordered(elem),
        Ty::Tuple(elements) | Ty::ShapedList(elements) => elements.iter().all(ordered),
        _ => false,
    }
}

/// Whether `left == right` is an equality of an `Optional[T]` with a `T`
/// or another `Optional[T]` the runtime compares natively (see
/// [`compare_link_ty`]): `T` a number, `str` or a container of those.
fn optional_equality(left: &Ty, right: &Ty) -> bool {
    let unwrap = |ty: &Ty| match ty {
        Ty::Optional(inner) => ((**inner).clone(), true),
        other => (other.clone(), false),
    };
    let ((l, l_optional), (r, r_optional)) = (unwrap(left), unwrap(right));
    let number = |ty: &Ty| matches!(ty, Ty::Int | Ty::Bool | Ty::Float | Ty::Num);
    let native = |ty: &Ty| {
        number(ty)
            || *ty == Ty::Str
            || (matches!(
                ty,
                Ty::List(_) | Ty::VarTuple(_) | Ty::Tuple(_) | Ty::Dict(_) | Ty::Set(_) | Ty::FrozenSet(_)
            ) && pycc_hir::builtins::value_comparable(ty))
    };
    (l_optional || r_optional) && native(&l) && native(&r) && (l == r || (number(&l) && number(&r)))
}
