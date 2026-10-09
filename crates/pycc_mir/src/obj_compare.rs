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
    /// type object (`src/ext/pycc_ext_module.c`).
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
    if left.ty() == Ty::Object || right.ty() == Ty::Object {
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
    let class = match (compiled, builtin) {
        (Some(name), _) => ObjIsInstanceClass::Compiled(name.clone()),
        (None, Some(builtin)) => ObjIsInstanceClass::Builtin(builtin),
        (None, None) => ObjIsInstanceClass::Object(Box::new(lower_expr(
            class_arg,
            scopes,
            classes,
            current_class,
        ))),
    };
    MirExpr::ObjIsInstance {
        value: Box::new(value),
        class,
    }
}

#[cfg(test)]
mod tests;
