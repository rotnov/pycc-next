//! The dataclass-synthesized `__eq__` and `__repr__` (#378, PR-18),
//! extracted from `class.rs` (#1337) when the `__repr__` body became
//! reusable for an inherited-method copy's receiver.

use crate::{HirExpr, HirItem, HirStmt, Ty};

/// #378 (PR-18): Synthesizes an `__eq__` method for a `@dataclass` class
/// from its (merged) field list. The synthesized method takes `self` and
/// `other` (both typed `Ty::Instance(class_name)`), and returns `bool` --
/// `True` if all fields are equal, `False` otherwise. The body uses a
/// series of `if self.<field> != other.<field>: return False` checks
/// followed by `return True`. The synthesis predates `and`/`or` lowering
/// (#1211), and a chain of early returns needs no field type to be
/// truth-testable, so it stays as it is. A zero-field dataclass's
/// `__eq__` always returns `True` (two instances of a fieldless dataclass
/// are always equal, matching CPython's PEP 557).
pub(super) fn synthesize_dataclass_eq(class_name: &str, fields: &[(String, Ty)]) -> HirItem {
    let self_ty = Ty::Instance(Box::new(class_name.to_string()));
    let params: Vec<(String, Ty)> = vec![
        ("self".to_string(), self_ty.clone()),
        ("other".to_string(), self_ty),
    ];
    let mut body: Vec<HirStmt> = Vec::new();
    for (name, _) in fields {
        // `if self.<field> != other.<field>: return False`
        body.push(HirStmt::If {
            test: HirExpr::Compare {
                op: crate::CmpOpKind::NotEq,
                left: Box::new(HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("self".to_string())),
                    attr: name.clone(),
                }),
                right: Box::new(HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("other".to_string())),
                    attr: name.clone(),
                }),
            },
            body: vec![HirStmt::Return(Some(HirExpr::BoolLiteral(false)))],
            orelse: Vec::new(),
        });
    }
    // `return True`
    body.push(HirStmt::Return(Some(HirExpr::BoolLiteral(true))));
    HirItem::Function {
        name: format!("{class_name}.__eq__"),
        params,
        return_ty: Ty::Bool,
        body,
    }
}

/// The body of a dataclass-synthesized `__repr__` that renders its class as
/// `display_name`: `return f"{display_name}(f1={self.f1}, ...)"`, or
/// `return "{display_name}()"` for a zero-field dataclass.
///
/// `synthesize_dataclass_repr` renders the defining class. `pycc_types`'
/// inherited-method copy pass (#1337, D-254) renders the receiver of a copy
/// instead, as CPython's `type(self).__qualname__` does for a subclass that
/// inherits the synthesized method.
#[must_use]
pub fn dataclass_repr_body(display_name: &str, fields: &[(String, Ty)]) -> Vec<HirStmt> {
    // Build the repr string as an f-string with literal and interpolation
    // parts. The codegen's f-string handling already converts each
    // interpolated value to a string via `to_str` and concatenates with
    // `pycc_rt_str_concat`.
    if fields.is_empty() {
        return vec![HirStmt::Return(Some(HirExpr::StringLiteral(format!(
            "{display_name}()"
        ))))];
    }
    let mut parts: Vec<crate::FStringPart> = Vec::new();
    parts.push(crate::FStringPart::Literal(format!("{display_name}(")));
    for (i, (name, _)) in fields.iter().enumerate() {
        if i > 0 {
            parts.push(crate::FStringPart::Literal(", ".to_string()));
        }
        parts.push(crate::FStringPart::Literal(format!("{name}=")));
        parts.push(crate::FStringPart::Interpolation(Box::new(
            HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("self".to_string())),
                attr: name.clone(),
            },
        )));
    }
    parts.push(crate::FStringPart::Literal(")".to_string()));
    vec![HirStmt::Return(Some(HirExpr::FString(parts)))]
}

/// #378 (PR-18): Synthesizes a `__repr__` method for a `@dataclass` class
/// from its (merged) field list. The synthesized method takes `self` and
/// returns a `str` of the form `ClassName(field1=..., field2=..., ...)`.
/// Each field value is converted to a string via f-string interpolation
/// (which routes through the existing `to_str` codegen for scalars). The
/// string is built by concatenating literal and interpolated parts using
/// `pycc_rt_str_concat` at codegen time (the f-string codegen already
/// does this).
///
/// For a zero-field dataclass, `__repr__` returns `"ClassName()"`.
pub(super) fn synthesize_dataclass_repr(class_name: &str, fields: &[(String, Ty)]) -> HirItem {
    let self_ty = Ty::Instance(Box::new(class_name.to_string()));
    let params: Vec<(String, Ty)> = vec![("self".to_string(), self_ty)];
    let body = dataclass_repr_body(class_name, fields);
    HirItem::Function {
        name: format!("{class_name}.__repr__"),
        params,
        return_ty: Ty::Str,
        body,
    }
}
