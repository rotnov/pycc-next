//! What an expression is statically known to be: a typed array of which
//! dtype and dimension count (numpy creation calls included), an int, a
//! float, a tuple of known length.

use super::*;

/// Looks a name's fact up in the scope, falling back to the module scope
/// for a name the scope does not bind.
pub(super) fn lookup<'f, T>(
    ctx: &'f Ctx,
    facts: &'f Facts,
    name: &str,
    get: impl Fn(&'f Facts) -> Option<T>,
) -> Option<T> {
    if facts.locals.contains(name) {
        get(facts)
    } else {
        get(&ctx.module)
    }
}

pub(super) fn nd_of_name(ctx: &Ctx, facts: &Facts, name: &str) -> Option<NdTy> {
    lookup(ctx, facts, name, |f| f.nd.get(name).copied())
}

/// The class of an instance-valued expression (a parameter typed with a
/// class, `self`).
pub(super) fn class_of(ctx: &Ctx, facts: &Facts, expr: &HirExpr) -> Option<String> {
    match expr {
        HirExpr::Name(name) => lookup(ctx, facts, name, |f| f.instances.get(name).cloned()),
        _ => None,
    }
}

/// The typed array an expression evaluates to, when that is known
/// statically (see the module doc).
pub(super) fn nd_of_value(ctx: &Ctx, facts: &Facts, expr: &HirExpr) -> Option<NdTy> {
    match expr {
        HirExpr::Name(name) => nd_of_name(ctx, facts, name),
        HirExpr::Call { callee, .. } if !facts.locals.contains(callee) => {
            ctx.fn_ret.get(callee).copied()
        }
        HirExpr::AttrGet { base, attr } => {
            let class = class_of(ctx, facts, base)?;
            ctx.class_attrs.get(&(class, attr.clone())).copied()
        }
        HirExpr::MethodCall { base, method, args } => {
            method_call_nd(ctx, facts, base, method, args, &[])
        }
        HirExpr::KeywordCall { call, keywords, .. } => match call.as_ref() {
            HirExpr::MethodCall { base, method, args } => {
                method_call_nd(ctx, facts, base, method, args, keywords)
            }
            _ => None,
        },
        HirExpr::Subscript { base, index } if is_scalar_index(index) => {
            let nd = nd_of_value(ctx, facts, base)?;
            (nd.ndim == 2).then_some(NdTy {
                dtype: nd.dtype,
                ndim: 1,
            })
        }
        _ => None,
    }
}

pub(super) fn is_scalar_index(index: &HirExpr) -> bool {
    !matches!(index, HirExpr::TupleLiteral(_) | HirExpr::Slice { .. })
}

pub(super) fn is_numpy_module(ctx: &Ctx, facts: &Facts, expr: &HirExpr) -> bool {
    matches!(expr, HirExpr::Name(name) if ctx.numpy.contains(name) && !facts.locals.contains(name))
}

pub(super) fn method_call_nd(
    ctx: &Ctx,
    facts: &Facts,
    base: &HirExpr,
    method: &str,
    args: &[HirExpr],
    keywords: &[(String, HirExpr)],
) -> Option<NdTy> {
    if is_numpy_module(ctx, facts, base) {
        return creation_nd(ctx, facts, method, args, keywords);
    }
    if let Some(nd) = nd_of_value(ctx, facts, base) {
        return (method == "copy" && args.is_empty() && keywords.is_empty()).then_some(nd);
    }
    let class = class_of(ctx, facts, base)?;
    let mangled = ctx.methods.get(&(class, method.to_string()))?;
    ctx.fn_ret.get(mangled).copied()
}

/// The dtype a dtype argument names: `np.float64`, `float`, `"int64"`.
pub(super) fn dtype_of(ctx: &Ctx, facts: &Facts, expr: &HirExpr) -> Option<NdDtype> {
    match expr {
        HirExpr::AttrGet { base, attr } if is_numpy_module(ctx, facts, base) => {
            NdDtype::from_attr(attr)
        }
        HirExpr::Name(name) if !facts.locals.contains(name) => match name.as_str() {
            "float" => Some(NdDtype::F64),
            "int" => Some(NdDtype::I64),
            "bool" => Some(NdDtype::Bool),
            _ => None,
        },
        HirExpr::StringLiteral(s) => match s.as_str() {
            "float64" | "f8" | "d" => Some(NdDtype::F64),
            "int64" | "i8" => Some(NdDtype::I64),
            "bool" | "?" => Some(NdDtype::Bool),
            _ => None,
        },
        _ => None,
    }
}

/// The number of dimensions a shape argument gives.
pub(super) fn shape_ndim(ctx: &Ctx, facts: &Facts, expr: &HirExpr) -> Option<u8> {
    let n = match expr {
        HirExpr::TupleLiteral(elements) | HirExpr::ListLiteral(elements) => elements.len(),
        other => match tuple_len(ctx, facts, other) {
            Some(n) => n,
            None if is_int(ctx, facts, other) => 1,
            None => return None,
        },
    };
    u8::try_from(n)
        .ok()
        .filter(|n| (1..=crate::ndarray::MAX_NDIM).contains(n))
}

pub(super) fn keyword<'e>(keywords: &'e [(String, HirExpr)], name: &str) -> Option<&'e HirExpr> {
    keywords.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

pub(super) fn creation_nd(
    ctx: &Ctx,
    facts: &Facts,
    method: &str,
    args: &[HirExpr],
    keywords: &[(String, HirExpr)],
) -> Option<NdTy> {
    // Only keywords whose meaning is known; an unknown one (`order=`,
    // `like=`) leaves the call an object.
    if keywords
        .iter()
        .any(|(k, _)| !matches!(k.as_str(), "dtype" | "shape"))
    {
        return None;
    }
    let dtype_kw = match keyword(keywords, "dtype") {
        Some(e) => Some(dtype_of(ctx, facts, e)?),
        None => None,
    };
    let nd = |dtype: NdDtype, ndim: u8| Some(NdTy { dtype, ndim });
    match method {
        "zeros" | "ones" | "empty" => {
            let (shape, dtype_pos) = match (args, keyword(keywords, "shape")) {
                ([shape], None) => (shape, None),
                ([shape, dtype], None) if dtype_kw.is_none() => {
                    (shape, Some(dtype_of(ctx, facts, dtype)?))
                }
                ([], Some(shape)) => (shape, None),
                _ => return None,
            };
            nd(
                dtype_kw.or(dtype_pos).unwrap_or(NdDtype::F64),
                shape_ndim(ctx, facts, shape)?,
            )
        }
        "full" => {
            let [shape, fill] = args else { return None };
            let dtype = match dtype_kw {
                Some(d) => d,
                None => match fill {
                    HirExpr::BoolLiteral(_) => NdDtype::Bool,
                    e if is_int(ctx, facts, e) => NdDtype::I64,
                    e if is_float(ctx, facts, e) => NdDtype::F64,
                    _ => return None,
                },
            };
            nd(dtype, shape_ndim(ctx, facts, shape)?)
        }
        "zeros_like" | "ones_like" | "empty_like" | "copy" => {
            let [source] = args else { return None };
            let source = nd_of_value(ctx, facts, source)?;
            nd(dtype_kw.unwrap_or(source.dtype), source.ndim)
        }
        "array" | "asarray" => {
            let [source] = args else { return None };
            if let Some(src) = nd_of_value(ctx, facts, source) {
                return nd(dtype_kw.unwrap_or(src.dtype), src.ndim);
            }
            let (depth, kind) = list_display(ctx, facts, source)?;
            let dtype = match dtype_kw {
                Some(d) => d,
                None => kind?,
            };
            nd(dtype, depth)
        }
        "arange" => {
            if args.is_empty() || args.len() > 3 {
                return None;
            }
            let dtype = match dtype_kw {
                Some(d) => d,
                None if args.iter().all(|a| is_int(ctx, facts, a)) => NdDtype::I64,
                None if args
                    .iter()
                    .all(|a| is_int(ctx, facts, a) || is_float(ctx, facts, a)) =>
                {
                    NdDtype::F64
                }
                None => return None,
            };
            nd(dtype, 1)
        }
        "linspace" => (dtype_kw.is_none() || dtype_kw == Some(NdDtype::F64))
            .then_some(NdTy {
                dtype: NdDtype::F64,
                ndim: 1,
            })
            .filter(|_| (2..=3).contains(&args.len())),
        _ => None,
    }
}

/// A list display's nesting depth and the dtype numpy infers for its
/// elements (`None` when some element's type is not evident). A ragged or
/// empty display is not a typed array.
pub(super) fn list_display(
    ctx: &Ctx,
    facts: &Facts,
    expr: &HirExpr,
) -> Option<(u8, Option<NdDtype>)> {
    let HirExpr::ListLiteral(elements) = expr else {
        return None;
    };
    if elements.is_empty() {
        return None;
    }
    if elements
        .iter()
        .all(|e| matches!(e, HirExpr::ListLiteral(_)))
    {
        let inner: Vec<(u8, Option<NdDtype>)> = elements
            .iter()
            .map(|e| list_display(ctx, facts, e))
            .collect::<Option<_>>()?;
        let lengths: HashSet<usize> = elements
            .iter()
            .map(|e| match e {
                HirExpr::ListLiteral(v) => v.len(),
                _ => 0,
            })
            .collect();
        if inner.iter().any(|(d, _)| *d != 1) || lengths.len() != 1 {
            return None;
        }
        let kinds: Vec<Option<NdDtype>> = inner.iter().map(|(_, k)| *k).collect();
        return Some((2, merge_kinds(&kinds)));
    }
    let kinds: Vec<Option<NdDtype>> = elements
        .iter()
        .map(|e| match e {
            HirExpr::BoolLiteral(_) => Some(NdDtype::Bool),
            e if is_int(ctx, facts, e) => Some(NdDtype::I64),
            e if is_float(ctx, facts, e) => Some(NdDtype::F64),
            _ => None,
        })
        .collect();
    Some((1, merge_kinds(&kinds)))
}

/// numpy's promotion of element kinds: all bool -> bool, all int -> int64,
/// any float among ints/floats -> float64; a bool mixed with numbers is
/// left alone (`None`).
pub(super) fn merge_kinds(kinds: &[Option<NdDtype>]) -> Option<NdDtype> {
    let kinds: Vec<NdDtype> = kinds.iter().copied().collect::<Option<_>>()?;
    if kinds.iter().all(|k| *k == NdDtype::Bool) {
        Some(NdDtype::Bool)
    } else if kinds.contains(&NdDtype::Bool) {
        None
    } else if kinds.iter().all(|k| *k == NdDtype::I64) {
        Some(NdDtype::I64)
    } else {
        Some(NdDtype::F64)
    }
}

pub(super) fn is_int(ctx: &Ctx, facts: &Facts, expr: &HirExpr) -> bool {
    use crate::BinOpKind as B;
    match expr {
        HirExpr::IntLiteral(_) => true,
        HirExpr::Name(name) => {
            lookup(ctx, facts, name, |f| f.ints.contains(name).then_some(())).is_some()
        }
        HirExpr::BinOp { op, left, right } | HirExpr::AugBinOp { op, left, right } => {
            !matches!(op, B::Div) && is_int(ctx, facts, left) && is_int(ctx, facts, right)
        }
        HirExpr::UnaryOp { op, operand } => {
            !matches!(op, crate::UnaryOpKind::Not) && is_int(ctx, facts, operand)
        }
        HirExpr::Call { callee, args } if !facts.locals.contains(callee) => match callee.as_str() {
            "len" | "int" => ctx.fn_int.contains(callee) || !ctx.functions.contains(callee),
            "abs" | "min" | "max" => !args.is_empty() && args.iter().all(|a| is_int(ctx, facts, a)),
            other => ctx.fn_int.contains(other),
        },
        HirExpr::Nd(nd) => {
            matches!(nd.op, NdOp::Size | NdOp::Ndim | NdOp::Dim(_))
                || (matches!(nd.op, NdOp::Get { .. }) && nd.nd.dtype == NdDtype::I64)
        }
        HirExpr::Subscript { base, index } => match base.as_ref() {
            HirExpr::AttrGet { base, attr } if attr == "shape" => {
                nd_of_value(ctx, facts, base).is_some()
                    && matches!(index.as_ref(), HirExpr::IntLiteral(_))
            }
            other => {
                nd_of_value(ctx, facts, other).is_some_and(|nd| nd.dtype == NdDtype::I64)
                    && match index.as_ref() {
                        HirExpr::TupleLiteral(t) => {
                            t.len()
                                == usize::from(nd_of_value(ctx, facts, other).map_or(0, |n| n.ndim))
                        }
                        _ => nd_of_value(ctx, facts, other).is_some_and(|n| n.ndim == 1),
                    }
            }
        },
        HirExpr::AttrGet { base, attr } => {
            matches!(attr.as_str(), "size" | "ndim") && nd_of_value(ctx, facts, base).is_some()
        }
        _ => false,
    }
}

pub(super) fn is_float(ctx: &Ctx, facts: &Facts, expr: &HirExpr) -> bool {
    use crate::BinOpKind as B;
    match expr {
        HirExpr::FloatLiteral(_) => true,
        HirExpr::Name(name) => {
            lookup(ctx, facts, name, |f| f.floats.contains(name).then_some(())).is_some()
        }
        HirExpr::BinOp { op, left, right } | HirExpr::AugBinOp { op, left, right } => {
            let l_num = is_float(ctx, facts, left) || is_int(ctx, facts, left);
            let r_num = is_float(ctx, facts, right) || is_int(ctx, facts, right);
            l_num
                && r_num
                && (matches!(op, B::Div)
                    || is_float(ctx, facts, left)
                    || is_float(ctx, facts, right))
        }
        HirExpr::UnaryOp { op, operand } => {
            matches!(op, crate::UnaryOpKind::USub | crate::UnaryOpKind::UAdd)
                && is_float(ctx, facts, operand)
        }
        HirExpr::Call { callee, .. } if callee == "float" && !facts.locals.contains(callee) => true,
        _ => false,
    }
}

/// The length of a tuple-valued expression, when evident.
pub(super) fn tuple_len(ctx: &Ctx, facts: &Facts, expr: &HirExpr) -> Option<usize> {
    match expr {
        HirExpr::TupleLiteral(elements) => Some(elements.len()),
        HirExpr::Name(name) => lookup(ctx, facts, name, |f| f.tuples.get(name).copied()),
        HirExpr::AttrGet { base, attr } if attr == "shape" => {
            nd_of_value(ctx, facts, base).map(|nd| usize::from(nd.ndim))
        }
        _ => None,
    }
}
