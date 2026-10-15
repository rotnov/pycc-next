//! Non-strict mode: a `collections.defaultdict` the native dict cannot hold,
//! built by CPython from its factory's source (`pycc_ext_obj_defaultdict`).

use crate::HirExpr;

/// Non-strict mode: the Python source of the factory of a
/// `collections.defaultdict` call (`pycc_ast::desugar::collections`'
/// `__pycc_defaultdict({}, proto, repr)`, or `__pycc_defaultdict_none({})`
/// for `defaultdict()`), for CPython to build the dict
/// (`pycc_ext_obj_defaultdict`) when the native one cannot hold its keys or
/// values: the type `repr` names (`<class 'list'>` is `list`), a `lambda`
/// returning the literal `proto`, or `None`. `None` for any other call or
/// factory.
#[must_use]
pub fn object_defaultdict_source(callee: &str, args: &[HirExpr]) -> Option<String> {
    let empty = |items: &HirExpr| {
        matches!(items, HirExpr::EmptyDict(_))
            || matches!(items, HirExpr::DictLiteral(pairs) if pairs.is_empty())
    };
    let (proto, repr) = match (callee, args) {
        ("__pycc_defaultdict_none", [items]) if empty(items) => return Some("None".to_string()),
        ("__pycc_defaultdict", [items, proto, HirExpr::StringLiteral(repr)]) if empty(items) => {
            (proto, repr.as_str())
        }
        _ => return None,
    };
    if let Some(class) = repr
        .strip_prefix("<class '")
        .and_then(|rest| rest.strip_suffix("'>"))
    {
        return matches!(class, "int" | "float" | "str" | "bool" | "list" | "dict" | "set")
            .then(|| class.to_string());
    }
    if repr != "<function <lambda>>" {
        return None;
    }
    let body = match proto {
        HirExpr::IntLiteral(n) => n.to_string(),
        HirExpr::BoolLiteral(b) => if *b { "True" } else { "False" }.to_string(),
        HirExpr::NoneLiteral => "None".to_string(),
        HirExpr::StringLiteral(text)
            if text.chars().all(|c| c.is_ascii_graphic() || c == ' ')
                && !text.contains(['\\', '\'']) =>
        {
            format!("'{text}'")
        }
        _ => return None,
    };
    Some(format!("lambda: {body}"))
}
