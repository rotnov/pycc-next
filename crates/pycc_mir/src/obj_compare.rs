//! Lowering of comparisons, identity tests and `isinstance` with a CPython
//! object operand (Part 1 of #1371) to [`MirExpr::ObjCompare`] and
//! [`MirExpr::ObjIsInstance`], and of a membership test against a CPython
//! object container (Part 2b of #1371) to [`MirExpr::ObjContains`].
//!
//! `pycc_types::foreign::compare` owns which shapes are admitted; this
//! module only routes an admitted shape to its run-time node. Both nodes
//! exist because the native lowerings would be wrong for an object, not
//! merely unimplemented: a native `MirExpr::Compare` answers `bool` where
//! CPython's rich comparison answers an arbitrary object, and the
//! `isinstance` fold (`class::lower_isinstance`) reads only the operand's
//! static type, so an object would fold to a constant `False`.

use super::MirExpr;
use super::expr::lower_expr;
use pycc_hir::{CmpOpKind, HirClassDef, HirExpr, Ty};
use std::collections::HashMap;

/// The builtin class an [`MirExpr::ObjIsInstance`] tests against when the
/// class argument is one of the scalar type names
/// (`pycc_hir::is_builtin_type_name`) or, since Part 7 of #1371, `list`,
/// `dict` or `tuple`, none of which has a binding of its own to evaluate:
/// codegen names CPython's own type object for each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjBuiltinClass {
    Int,
    Str,
    Float,
    Bool,
    List,
    Dict,
    Tuple,
}

impl ObjBuiltinClass {
    /// The class a builtin type name denotes, or `None` for any other name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "int" => Some(Self::Int),
            "str" => Some(Self::Str),
            "float" => Some(Self::Float),
            "bool" => Some(Self::Bool),
            "list" => Some(Self::List),
            "dict" => Some(Self::Dict),
            "tuple" => Some(Self::Tuple),
            _ => None,
        }
    }

    /// The selector `pycc_ext_obj_isinstance` reads to pick the builtin
    /// type object (`src/ext/module/`).
    pub fn shim_code(self) -> u64 {
        match self {
            Self::Int => 0,
            Self::Str => 1,
            Self::Float => 2,
            Self::Bool => 3,
            Self::List => 4,
            Self::Dict => 5,
            Self::Tuple => 6,
        }
    }
}

/// The class operand of an [`MirExpr::ObjIsInstance`].
#[derive(Debug, Clone, PartialEq)]
pub enum ObjIsInstanceClass {
    /// A builtin scalar type name.
    Builtin(ObjBuiltinClass),
    /// An object-typed expression -- a foreign class such as `mod.Cls`.
    Object(Box<MirExpr>),
    /// A class compiled in this module, by its name (Part 7 of #1371).
    /// It has no value to evaluate: the artifact's generated
    /// `pycc_ext_compiled_class_isinstance` answers it against the host type
    /// object of every published class whose MRO contains it.
    Compiled(String),
    /// A tuple of classes (`isinstance(o, (dict, list))`), nested tuples
    /// flattened: every class expression is evaluated first, as CPython
    /// builds the tuple, then the classes are tested in order until one
    /// answers `True`.
    AnyOf(Vec<ObjIsInstanceClass>),
}

impl ObjIsInstanceClass {
    /// The class expressions it evaluates, in order.
    pub fn objects(&self) -> Vec<&MirExpr> {
        match self {
            Self::Object(class) => vec![class],
            Self::AnyOf(classes) => classes.iter().flat_map(Self::objects).collect(),
            Self::Builtin(_) | Self::Compiled(_) => Vec::new(),
        }
    }
}

/// `Ok(ObjContains)` for `left in right` / `left not in right` whose
/// container `right` is a CPython object, `Ok(ObjCompare)` when either
/// lowered operand of any other `left op right` is a CPython object, so the
/// native comparison lowering never sees one; `Err` hands the operands back.
pub(super) fn lower_object_compare(
    op: CmpOpKind,
    left: MirExpr,
    right: MirExpr,
) -> Result<MirExpr, Box<(MirExpr, MirExpr)>> {
    if matches!(op, CmpOpKind::In | CmpOpKind::NotIn) && right.ty() == Ty::Object {
        return Ok(MirExpr::ObjContains {
            negate: op == CmpOpKind::NotIn,
            item: Box::new(left),
            container: Box::new(right),
        });
    }
    // An object in a native `str`, or in a native frozenset of scalars: the
    // container is boxed into a CPython one.
    // Non-strict mode: so is a native list, tuple or set of scalars
    // (`pycc_types`' `scalar_collection`), as its CPython copy.
    let scalar = |t: &Ty| matches!(t, Ty::Int | Ty::Float | Ty::Bool | Ty::Str);
    if matches!(op, CmpOpKind::In | CmpOpKind::NotIn)
        && left.ty() == Ty::Object
        && (right.ty() == Ty::Str
            || matches!(right.ty(), Ty::FrozenSet(element)
                | Ty::List(element) | Ty::VarTuple(element) | Ty::Set(element) if scalar(&element))
            || matches!(right.ty(), Ty::Tuple(elements) if elements.iter().all(scalar)))
    {
        return Ok(MirExpr::ObjContains {
            negate: op == CmpOpKind::NotIn,
            item: Box::new(left),
            container: Box::new(MirExpr::ObjectBox(Box::new(right))),
        });
    }
    if left.ty() == Ty::Object || right.ty() == Ty::Object {
        // A `None` operand of a rich comparison (`o == None`) is CPython's
        // `Py_None`; an identity test keeps its own `None` operand.
        let (left, right) = if matches!(op, CmpOpKind::Is | CmpOpKind::IsNot) {
            // v0.9: a compiled instance against an object is compared as
            // its carrier, the object CPython code holds for it.
            let carried = |side: MirExpr| {
                if matches!(side.ty(), Ty::Instance(_)) {
                    MirExpr::ObjectBox(Box::new(side))
                } else {
                    side
                }
            };
            (carried(left), carried(right))
        } else {
            (
                crate::obj_arith::display_element(left),
                crate::obj_arith::display_element(right),
            )
        };
        Ok(MirExpr::ObjCompare {
            op,
            left: Box::new(left),
            right: Box::new(right),
        })
    } else {
        Err(Box::new((left, right)))
    }
}

/// Lowers `isinstance(value, class_arg)` once `value` is known to be a
/// CPython object.
pub(super) fn lower_object_isinstance(
    value: MirExpr,
    class_arg: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    MirExpr::ObjIsInstance {
        value: Box::new(value),
        class: isinstance_class(class_arg, scopes, classes, current_class),
    }
}

/// The class operand `class_arg` of an object `isinstance` denotes.
fn isinstance_class(
    class_arg: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> ObjIsInstanceClass {
    if let HirExpr::TupleLiteral(elements) = class_arg {
        let mut flat = Vec::new();
        for element in elements {
            match isinstance_class(element, scopes, classes, current_class) {
                ObjIsInstanceClass::AnyOf(inner) => flat.extend(inner),
                class => flat.push(class),
            }
        }
        return ObjIsInstanceClass::AnyOf(flat);
    }
    // A local or parameter spelled like a builtin or a compiled class
    // shadows it, as in CPython: it is then the evaluated class argument.
    // A module function spelled like one shadows it as well; the checker
    // refuses that guard (`check_object_isinstance`), so this only keeps
    // the two layers naming the same class argument.
    let unshadowed = match class_arg {
        HirExpr::Name(name)
            if !scopes.iter().any(|scope| {
                scope.contains_key(name) || scope.contains_key(&format!("$fn:{name}"))
            }) =>
        {
            Some(name)
        }
        _ => None,
    };
    // A module-level class spelled like a builtin (`class int: ...`)
    // shadows the builtin too, so the compiled class is consulted first
    // (#1476: `pycc_hir::isinstance_narrow_target` narrows in the same
    // order, so a guard and the narrowing it licenses name one class).
    let compiled = unshadowed.filter(|name| classes.contains_key(*name));
    let builtin = unshadowed.and_then(|name| ObjBuiltinClass::from_name(name));
    match (compiled, builtin) {
        (Some(name), _) => ObjIsInstanceClass::Compiled(name.clone()),
        (None, Some(builtin)) => ObjIsInstanceClass::Builtin(builtin),
        (None, None) => ObjIsInstanceClass::Object(Box::new(lower_expr(
            class_arg,
            scopes,
            classes,
            current_class,
        ))),
    }
}

#[cfg(test)]
mod tests;
