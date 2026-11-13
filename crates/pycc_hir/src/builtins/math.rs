//! The `math` functions of `float` arguments, and `math.log` of one
//! argument, by runtime symbol.

use super::{BuiltinSig, is_intlike, sig};
use crate::Ty;

/// The runtime symbol of a `math` function of `float` arguments (an `int`
/// argument is converted to `float` at the call) and its arity.
pub(super) fn float_math(name: &str) -> Option<(&'static str, usize)> {
    Some(match name {
        "math.sqrt" => ("pycc_rt_math_sqrt", 1),
        "math.exp" => ("pycc_rt_math_exp", 1),
        "math.sin" => ("pycc_rt_math_sin", 1),
        "math.cos" => ("pycc_rt_math_cos", 1),
        "math.tan" => ("pycc_rt_math_tan", 1),
        "math.asin" => ("pycc_rt_math_asin", 1),
        "math.acos" => ("pycc_rt_math_acos", 1),
        "math.atan" => ("pycc_rt_math_atan", 1),
        "math.fabs" => ("pycc_rt_math_fabs", 1),
        "math.atan2" => ("pycc_rt_math_atan2", 2),
        "math.hypot" => ("pycc_rt_math_hypot", 2),
        "math.copysign" => ("pycc_rt_math_copysign", 2),
        _ => return None,
    })
}

/// `math.log`/`log2`/`log10` of one argument: an `int` is taken exactly.
pub(super) fn math_log(name: &str, arg: &Ty) -> Option<BuiltinSig> {
    let (float_symbol, int_symbol) = match name {
        "math.log" => ("pycc_rt_math_log", "pycc_rt_math_log_int"),
        "math.log2" => ("pycc_rt_math_log2", "pycc_rt_math_log2_int"),
        "math.log10" => ("pycc_rt_math_log10", "pycc_rt_math_log10_int"),
        _ => return None,
    };
    Some(match arg {
        // An `int | float` argument converts as `float(x)` does.
        Ty::Float | Ty::Num => sig(float_symbol, &[Ty::Float], Ty::Float),
        a if is_intlike(a) => sig(int_symbol, &[Ty::Int], Ty::Float),
        _ => return None,
    })
}
