//! Item and attribute stores: `HirStmt::DictSet` (`base[key] = value` on a
//! dict, list, memoryview or CPython object) and `HirStmt::AttrSet`
//! (`base.attr = value`), split out of `lower_stmt`.

use super::super::{
    HirClassDef, MirExpr, MirStmt, class_def_of, lookup, lower_expr, mro_attrs, mro_class_def,
};
use pycc_hir::{HirExpr, Ty};
use std::collections::HashMap;

/// `dict[key] = value`, where `dict` names a dict, list, memoryview or
/// CPython object.
pub(super) fn lower_item_store(
    dict: &str,
    key: &HirExpr,
    value: &HirExpr,
    scopes: &mut [HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirStmt {
    let key = lower_expr(key, scopes, classes, current_class);
    let value = lower_expr(value, scopes, classes, current_class);
    let base_ty = lookup(scopes, dict);
    // Non-strict mode (#3): `pycc_types` admitted an item store on a
    // CPython object, which is `PyObject_SetItem`.
    if base_ty == Ty::Object {
        return MirStmt::ObjSetItem {
            base: MirExpr::Name {
                name: dict.to_owned(),
                ty: Ty::Object,
            },
            key,
            value: crate::obj_arith::object_arg(value),
        };
    }
    if let Ty::List(_) = base_ty {
        MirStmt::ListSet {
            base: MirExpr::Name {
                name: dict.to_owned(),
                ty: base_ty,
            },
            index: crate::index_operand(key, crate::IndexUse::List),
            value,
        }
    } else if base_ty == Ty::MemoryView {
        MirStmt::BufferSet {
            base: MirExpr::Name {
                name: dict.to_owned(),
                ty: Ty::MemoryView,
            },
            index: key,
            value,
        }
    } else if let Some(fused) = crate::dict_add::fused_dict_add(dict, &base_ty, &key, &value) {
        MirStmt::ExprStmt(fused)
    } else {
        MirStmt::DictSet {
            dict: dict.to_owned(),
            key,
            value,
        }
    }
}

/// D-154 (Part 1 of #375): `base.attr = value`, resolved to a
/// compile-time slot index exactly like `MirExpr::AttrGet`.
/// #377: if `attr` is a `@property` with a setter, the assignment is
/// rewritten to an ordinary `MirStmt::ExprStmt(MirExpr::Call)` to
/// the setter's mangled name (with `base` as `self` and `value` as
/// the setter's parameter), reusing the existing method-call/codegen
/// infrastructure with no new MIR/codegen variant. A read-only
/// property (no setter) never reaches here -- `pycc_types::check`
/// rejects it with `T0044` before MIR lowering runs.
pub(super) fn lower_attr_store(
    base: &HirExpr,
    attr: &String,
    value: &HirExpr,
    scopes: &mut [HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirStmt {
    let base = lower_expr(base, scopes, classes, current_class);
    let value = lower_expr(value, scopes, classes, current_class);
    // #1457 (Part 2 of #1443): a CPython object has no
    // `HirClassDef`, so the store stays String-keyed. It must come
    // before `class_def_of`, which panics on a `Ty::Object` base.
    if base.ty() == Ty::Object {
        return MirStmt::ObjAttrSet {
            base,
            attr: attr.clone(),
            value,
        };
    }
    let class_def = class_def_of(&base, classes);
    // #432: walk the MRO for property lookup first (matching
    // `AttrGet`'s own MRO walk), then for regular attribute slots
    // using the flat MRO layout.
    for mro_class in &class_def.mro {
        let mro_def = mro_class_def(mro_class, classes);
        if let Some(prop) = mro_def.properties.iter().find(|p| p.name == *attr) {
            let setter = prop.setter.as_ref().unwrap_or_else(|| {
                panic!(
                    "pycc_mir: internal error: property `{attr}` on class `{mro_class}` \
                     has no setter -- pycc_types::check should have rejected this \
                     assignment before it reached pycc_mir"
                )
            });
            let setter = crate::receiver_exact::exact_callee(
                &class_def.name,
                mro_class,
                setter.clone(),
                scopes,
                classes,
            );
            let ty = lookup(scopes, &format!("$fn:{setter}"));
            return MirStmt::ExprStmt(MirExpr::Call {
                callee: setter,
                args: vec![base, value],
                ty,
            });
        }
    }
    let flat_attrs = mro_attrs(class_def, classes);
    let (slot, (_, slot_ty)) = flat_attrs
        .iter()
        .enumerate()
        .find(|(_, (name, _))| name == attr)
        .unwrap_or_else(|| {
            panic!(
                "pycc_mir: internal error: attribute `{attr}` not declared on class `{}` \
                 or any base in its MRO -- pycc_types::check should have rejected this \
                 HIR before it reached pycc_mir",
                class_def.name
            )
        });
    // #627: `obj.attr = <bool>` where `attr` is declared `int`.
    // `pycc_types::is_assignable` accepts this -- `int` admits
    // `bool` as a subtype at a checked boundary
    // (`docs/TYPE_SYSTEM.md`) -- and an attribute store is such a
    // boundary, exactly like the `AnnAssign` arm above. The slot
    // holds D-141-encoded `int` words, so an unencoded `bool`
    // word lands there as a raw `1`/`0`: `1` reads back as the
    // smallint `0`, and `0` is not a valid encoded word at all,
    // aborting the next read with `pycc_rt: invalid encoded int
    // word 0x0`. Widen through `MirExpr::IntBoundary`, the
    // mechanism D-141 mandates ("MIR represents an annotated
    // initializer boundary with `MirExpr::IntBoundary`, not
    // synthetic arithmetic"), which preserves the runtime `True`/
    // `False` identity CPython prints. The declared type comes
    // from the same `flat_attrs` tuple the slot index did, never
    // from a fresh lookup on the class's own `attrs`, because an
    // MRO re-declaration can make the two differ (#432).
    //
    // Reporting `Ty::Int` for the stored value additionally makes
    // codegen's `MirStmt::AttrSet` release gate fire, so a bigint
    // already in the slot is released instead of leaked -- D-180
    // Consequences item 6, corrected by D-187.
    // Part 2 of #1387: a native value stored into an
    // `object`-typed slot is boxed the same way.
    let value = if *slot_ty == Ty::Int && value.ty() == Ty::Bool {
        MirExpr::IntBoundary(Box::new(value))
    } else if *slot_ty == Ty::Num && value.ty() != Ty::Num {
        // An `int | float` attribute holds `int | float` words, and the
        // wrapped value's `Ty::Num` makes the release gate fire too.
        MirExpr::NumWrap(Box::new(value))
    } else {
        crate::object_box::box_into(value, slot_ty)
    };
    MirStmt::AttrSet { base, slot, value }
}
