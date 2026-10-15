//! An `IntEnum` member in a position that reads an `int`
//! (`pycc_hir::int_enum`): lowered to the member's `value`.

use super::*;

/// The class of `ty` when it is an instance of an `IntEnum` class.
pub(crate) fn int_enum_class<'a>(
    ty: &Ty,
    classes: &'a HashMap<String, HirClassDef>,
) -> Option<&'a HirClassDef> {
    let Ty::Instance(class) = ty else {
        return None;
    };
    classes
        .get(class.as_str())
        .filter(|def| pycc_hir::int_enum::is_int_enum_class(def))
}

/// `lowered` (the lowering of `hir`), or `hir.value` lowered when it is an
/// `IntEnum` member.
pub(crate) fn as_int(
    hir: &HirExpr,
    lowered: MirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    if int_enum_class(&lowered.ty(), classes).is_none() {
        return lowered;
    }
    lower_expr(
        &HirExpr::AttrGet {
            base: Box::new(hir.clone()),
            attr: "value".to_string(),
        },
        scopes,
        classes,
        current_class,
    )
}

/// Whether a comparison between operands of types `a` and `b` reads an
/// `IntEnum` member among them as its `int` value: a number on the other
/// side, or a member of another `IntEnum` class.
pub(crate) fn compares_as_ints(a: &Ty, b: &Ty, classes: &HashMap<String, HirClassDef>) -> bool {
    let numeric = |t: &Ty| matches!(t, Ty::Int | Ty::Float | Ty::Bool);
    match (int_enum_class(a, classes), int_enum_class(b, classes)) {
        (Some(x), Some(y)) => x.name != y.name,
        (Some(_), None) => numeric(b),
        (None, Some(_)) => numeric(a),
        (None, None) => false,
    }
}

/// `hir` lowered where an `int` is read (a `range()` bound): an `IntEnum`
/// member is its `value`.
pub(crate) fn lower_as_int(
    hir: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let lowered = lower_expr(hir, scopes, classes, current_class);
    as_int(hir, lowered, scopes, classes, current_class)
}
