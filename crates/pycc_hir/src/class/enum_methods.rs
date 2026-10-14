//! The methods CPython's `enum` gives an enum class, synthesized for
//! `lower_enum_class`:
//!
//! * `__repr__`: `<Color.RED: 1>` (the value's own repr: `<K.A: 'a'>`);
//! * `__str__`: `Color.RED` for an `Enum`, the value for an `IntEnum`
//!   (`str(Pri.HIGH)` is `2` since Python 3.11) or a `StrEnum`;
//! * an `IntEnum`'s ordering `__lt__`/`__le__`/`__gt__`/`__ge__`, comparing
//!   the values;
//! * the static method [`LOOKUP`]: `Color(value)` (rewritten to it by
//!   `pycc_ast::desugar::enums`) returns the first member with that value or
//!   raises CPython's `ValueError: 4 is not a valid Color`.
//!
//! An enum member is a module-lifetime singleton, so `==`, `is` and the
//! identity hash need no method.

use crate::class::EnumMemberValue;
use crate::{CmpOpKind, FStringPart, HirExpr, HirItem, HirStmt, Ty};

/// The value-lookup static method `Color(value)` calls.
pub const LOOKUP: &str = "__pycc_lookup__";

/// The synthesized methods: `(methods, static_methods, items)`.
pub(super) type EnumMethods = (Vec<(String, String)>, Vec<(String, String)>, Vec<HirItem>);

fn self_attr(attr: &str) -> HirExpr {
    HirExpr::AttrGet {
        base: Box::new(HirExpr::Name("self".to_string())),
        attr: attr.to_string(),
    }
}

fn call(callee: &str, arg: HirExpr) -> HirExpr {
    HirExpr::Call {
        callee: callee.to_string(),
        args: vec![arg],
    }
}

fn method(class: &str, name: &str, params: Vec<(String, Ty)>, return_ty: Ty, value: HirExpr) -> HirItem {
    HirItem::Function {
        name: format!("{class}.{name}"),
        params,
        return_ty,
        body: vec![HirStmt::Return(Some(value))],
    }
}

/// Builds the methods of enum class `class` (marker base `marker`, member
/// value type `value_ty`) with members `members`.
pub(super) fn enum_methods(
    class: &str,
    marker: &str,
    value_ty: &Ty,
    members: &[(String, EnumMemberValue)],
) -> EnumMethods {
    let self_ty = Ty::Instance(Box::new(class.to_string()));
    let receiver = || vec![("self".to_string(), self_ty.clone())];
    let mut items = Vec::new();
    let mut methods = Vec::new();
    let mut add = |name: &str, item: HirItem, methods: &mut Vec<(String, String)>| {
        methods.push((name.to_string(), format!("{class}.{name}")));
        items.push(item);
    };
    let repr = HirExpr::FString(vec![
        FStringPart::Literal(format!("<{class}.")),
        FStringPart::Interpolation(Box::new(self_attr("name"))),
        FStringPart::Literal(": ".to_string()),
        FStringPart::Interpolation(Box::new(call("repr", self_attr("value")))),
        FStringPart::Literal(">".to_string()),
    ]);
    add(
        "__repr__",
        method(class, "__repr__", receiver(), Ty::Str, repr),
        &mut methods,
    );
    let text = match marker {
        "IntEnum" => call("str", self_attr("value")),
        "StrEnum" => self_attr("value"),
        _ => HirExpr::FString(vec![
            FStringPart::Literal(format!("{class}.")),
            FStringPart::Interpolation(Box::new(self_attr("name"))),
        ]),
    };
    add(
        "__str__",
        method(class, "__str__", receiver(), Ty::Str, text),
        &mut methods,
    );
    // `Enum.__hash__` is `hash(self._name_)`; an `IntEnum`/`StrEnum` member
    // hashes as its value (`int.__hash__`/`str.__hash__` come first in the
    // MRO), so a set orders its members as CPython's does.
    let hashed = if marker == "Enum" { "name" } else { "value" };
    add(
        "__hash__",
        method(class, "__hash__", receiver(), Ty::Int, call("hash", self_attr(hashed))),
        &mut methods,
    );
    if marker == "IntEnum" {
        for (name, op) in [
            ("__lt__", CmpOpKind::Lt),
            ("__le__", CmpOpKind::LtE),
            ("__gt__", CmpOpKind::Gt),
            ("__ge__", CmpOpKind::GtE),
        ] {
            let mut params = receiver();
            params.push(("other".to_string(), self_ty.clone()));
            let compare = HirExpr::Compare {
                op,
                left: Box::new(self_attr("value")),
                right: Box::new(HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("other".to_string())),
                    attr: "value".to_string(),
                }),
            };
            add(name, method(class, name, params, Ty::Bool, compare), &mut methods);
        }
    }
    // `Color(value)`: the first member whose value equals `value`.
    let mut body: Vec<HirStmt> = members
        .iter()
        .map(|(member, value)| HirStmt::If {
            test: HirExpr::Compare {
                op: CmpOpKind::Eq,
                left: Box::new(HirExpr::Name("value".to_string())),
                right: Box::new(match value {
                    EnumMemberValue::Int(v) => HirExpr::IntLiteral(*v),
                    EnumMemberValue::Str(s) => HirExpr::StringLiteral(s.clone()),
                }),
            },
            body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
                base: Box::new(HirExpr::Name(class.to_string())),
                attr: member.clone(),
            }))],
            orelse: Vec::new(),
        })
        .collect();
    body.push(HirStmt::Raise {
        exc: Some(call(
            "ValueError",
            HirExpr::FString(vec![
                FStringPart::Interpolation(Box::new(call(
                    "repr",
                    HirExpr::Name("value".to_string()),
                ))),
                FStringPart::Literal(format!(" is not a valid {class}")),
            ]),
        )),
        cause: None,
    });
    items.push(HirItem::Function {
        name: format!("{class}.{LOOKUP}"),
        params: vec![("value".to_string(), value_ty.clone())],
        return_ty: self_ty,
        body,
    });
    let static_methods = vec![(LOOKUP.to_string(), format!("{class}.{LOOKUP}"))];
    (methods, static_methods, items)
}
