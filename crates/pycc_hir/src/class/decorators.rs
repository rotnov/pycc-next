//! Method and class decorator classification.

use super::*;

/// Classifies a method's decorator list (#377, #432). Returns:
/// - `Regular { is_override: false }` if the list is empty.
/// - `Regular { is_override: true }` if the single decorator is `@override`
///   (a bare name `override`, PEP 698).
/// - `PropertyGetter` if the single decorator is `@property` (a bare name
///   `property`).
/// - `PropertySetter` if the single decorator is `@<name>.setter` (an
///   attribute access on a bare name, where the attribute is `"setter"`).
///   The method's own source name must match `<name>` -- a mismatch
///   (e.g. `@x.setter def y(self, v): ...`) is rejected with `C0001`,
///   matching the common Python idiom where the setter's method name
///   matches the property name.
/// - Any other decorator shape (multiple decorators, a call-shaped
///   decorator, a different attribute name) is rejected with `C0001`,
///   preserving the pre-#377 "method decorators are not supported yet"
///   diagnostic for everything outside `@property`/`@<name>.setter`/
///   `@override`.
pub(crate) fn classify_decorator(
    decorator_list: &[Decorator],
    method_name: &str,
    range: std::ops::Range<u32>,
) -> Result<MethodKind, Diagnostic> {
    if decorator_list.is_empty() {
        return Ok(MethodKind::Regular { is_override: false });
    }
    if decorator_list.len() > 1 {
        return Err(unsupported(
            "method decorators are not supported yet",
            range,
        ));
    }
    let decorator = &decorator_list[0];
    match &decorator.expression {
        // `@property` -- a bare name `property`.
        Expr::Name(name) if name.id.as_str() == "property" => Ok(MethodKind::PropertyGetter {
            prop_name: method_name.to_string(),
        }),
        // `@override` -- a bare name `override` (PEP 698). The method is
        // still a regular method; `lower_class` verifies the method name
        // exists in at least one base class.
        Expr::Name(name) if name.id.as_str() == "override" => {
            Ok(MethodKind::Regular { is_override: true })
        }
        // `@staticmethod` (#436) -- a bare name `staticmethod`.
        Expr::Name(name) if name.id.as_str() == "staticmethod" => Ok(MethodKind::StaticMethod),
        // `@classmethod` (#436) -- a bare name `classmethod`.
        Expr::Name(name) if name.id.as_str() == "classmethod" => Ok(MethodKind::ClassMethod),
        // `@abstractmethod` (#380, PR-20, PEP 3119) -- a bare name
        // `abstractmethod`. Recognized as a bare name without requiring
        // `from abc import abstractmethod`, matching the
        // `Final`/`Annotated`/`Enum` precedent.
        Expr::Name(name) if name.id.as_str() == "abstractmethod" => Ok(MethodKind::AbstractMethod),
        // `@<name>.setter` -- an attribute access on a bare name, where
        // the attribute is `"setter"`.
        Expr::Attribute(attr) => {
            let Expr::Name(base_name) = attr.value.as_ref() else {
                return Err(unsupported(
                    "method decorators are not supported yet",
                    range,
                ));
            };
            if attr.attr.as_str() != "setter" {
                return Err(unsupported(
                    "method decorators are not supported yet",
                    range,
                ));
            }
            let prop_name = base_name.id.as_str().to_string();
            // The decorator name must match the method's own source name
            // (e.g. `@x.setter def x(self, v): ...`, not `@x.setter def
            // y(self, v): ...`). This is the overwhelmingly common idiom;
            // the uncommon mismatch case is rejected rather than
            // silently supporting a shape this PR does not need.
            if prop_name != method_name {
                return Err(unsupported(
                    format!(
                        "a `@{prop_name}.setter` decorator must decorate a method named \
                         `{prop_name}`, not `{method_name}`"
                    ),
                    range,
                ));
            }
            Ok(MethodKind::PropertySetter { prop_name })
        }
        _ => Err(unsupported(
            "method decorators are not supported yet",
            range,
        )),
    }
}

/// PEP 3129/557/681/544 (#378/#380, PR-18/PR-20): classifies a class's
/// decorator list to determine whether the class is a `@dataclass` (PEP
/// 557), a `@dataclass_transform(...)` (PEP 681), or a
/// `@runtime_checkable` (PEP 544) class. Returns a `ClassDecoratorInfo`
/// struct.
///
/// Any other class decorator shape (a different bare name, an
/// attribute-access decorator, `@dataclass(frozen=True)` with options) is
/// rejected with `C0001`, preserving the pre-#378 "class decorators are
/// not supported yet" diagnostic for everything outside
/// `@dataclass`/`@dataclass_transform()`/`@runtime_checkable`.
pub(crate) fn classify_class_decorator(
    decorator_list: &[Decorator],
    range: std::ops::Range<u32>,
) -> Result<ClassDecoratorInfo, Diagnostic> {
    let mut info = ClassDecoratorInfo {
        is_dataclass: false,
        runtime_checkable: false,
    };
    if decorator_list.is_empty() {
        return Ok(info);
    }
    if decorator_list.len() > 2 {
        return Err(unsupported(
            "more than two class decorators are not supported yet",
            range,
        ));
    }
    for decorator in decorator_list {
        match &decorator.expression {
            // `@dataclass` -- a bare name `dataclass` (PEP 557).
            Expr::Name(name) if name.id.as_str() == "dataclass" => {
                info.is_dataclass = true;
            }
            // `@runtime_checkable` -- a bare name `runtime_checkable`
            // (PEP 544, #380, PR-20). Recognized as a bare name without
            // requiring `from typing import runtime_checkable`, matching
            // the `Final`/`Annotated`/`Enum` precedent.
            Expr::Name(name) if name.id.as_str() == "runtime_checkable" => {
                info.runtime_checkable = true;
            }
            // `@dataclass_transform(...)` -- a call whose callee is the
            // bare name `dataclass_transform` (PEP 681). The keyword
            // arguments are accepted but ignored -- pycc's dataclass
            // implementation always generates `__init__`/`__eq__`/`__repr__`,
            // matching the defaults.
            Expr::Call(call) => {
                let Expr::Name(name) = call.func.as_ref() else {
                    return Err(unsupported("class decorators are not supported yet", range));
                };
                if name.id.as_str() == "dataclass_transform" {
                    info.is_dataclass = true;
                } else if name.id.as_str() == "dataclass" {
                    // `@dataclass(frozen=True)` etc. -- rejected with C0001.
                    return Err(unsupported(
                        "`@dataclass` with options is not supported yet -- only a bare \
                         `@dataclass` is supported in this version",
                        range,
                    ));
                } else {
                    return Err(unsupported("class decorators are not supported yet", range));
                }
            }
            _ => {
                return Err(unsupported("class decorators are not supported yet", range));
            }
        }
    }
    // #380 (PR-20): `@dataclass` combined with `@runtime_checkable` is
    // rejected — a protocol class cannot be a dataclass (a protocol is a
    // compile-time-only interface description, not an instantiable class
    // with fields).
    if info.is_dataclass && info.runtime_checkable {
        return Err(unsupported(
            "`@dataclass` combined with `@runtime_checkable` is not supported -- a protocol \
             class cannot be a dataclass",
            range,
        ));
    }
    Ok(info)
}
