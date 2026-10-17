//! A byte buffer's `append` and `extend` (`crate::ndarray`): builtins that
//! store a native `int` (or list of them) straight into the `bytearray`,
//! and are the method call otherwise.

use super::resolve_call;
use crate::Ty;

/// `b.append(x)` on a byte buffer (`crate::ndarray`): `x` stored natively
/// when it is an `int`, the method called otherwise.
pub const BYTEARRAY_APPEND: &str = "__pycc_ba_append";
/// `b.extend(xs)` on a byte buffer: a native list of `int`s stored
/// natively, the method called otherwise.
pub const BYTEARRAY_EXTEND: &str = "__pycc_ba_extend";

/// The method call a byte buffer builtin (`BYTEARRAY_APPEND`,
/// `BYTEARRAY_EXTEND`) of `args` (the buffer, then the argument) stands
/// for, when its argument is not one the builtin stores natively: then the
/// call is that method call on the `bytearray` object.
#[must_use]
pub fn byte_buffer_method_call(callee: &str, args: &[crate::HirExpr], tys: &[Ty]) -> Option<crate::HirExpr> {
    let method = match callee {
        BYTEARRAY_APPEND => "append",
        BYTEARRAY_EXTEND => "extend",
        _ => return None,
    };
    let [base, arg] = args else {
        return None;
    };
    if matches!(resolve_call(callee, tys), Some(Ok(_))) {
        return None;
    }
    Some(crate::HirExpr::MethodCall {
        base: Box::new(base.clone()),
        method: method.to_string(),
        args: vec![arg.clone()],
    })
}
