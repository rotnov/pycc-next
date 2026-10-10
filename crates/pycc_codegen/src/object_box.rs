//! A native value boxed into an `object` slot (Part 2 of #1387, D-258's
//! boxing amendment).
//!
//! `pycc_types::object_box::admits` lets a native `int`, `float`, `bool`,
//! `str`, compiled-instance or `None` value into an `object` slot at four
//! seams. Each reaches [`emit_boxed`]:
//!
//! - a binding, a rebinding or an attribute store, through
//!   `MirExpr::ObjectBox` (`pycc_mir` knows those slots' types);
//! - a call argument, from `build_call_to_with_leading_args`, the one
//!   marshalling helper every user-function call shape shares (a plain
//!   call, a constructor, a method, static, class or `super()` call, a
//!   property setter, and an explicit `x.__eq__(y)`), when the parameter
//!   is `object` and the argument is not. An in-module comparison operator
//!   (`a < b`, `a == b` between instances) reaches no `object` parameter:
//!   `pycc_types` types it without user dunder dispatch (`T0021`);
//! - a returned value, from the `return` path, when the function returns
//!   `object` and the value is not.
//!
//! The value is the CPython object CPython itself would hold: the scalar
//! packers (`foreign_pack::emit_pack`) build an `int`, `float`, `bool` or
//! `str`, a compiled instance crosses as its `PyccExtInstance` carrier (so
//! its identity survives a second crossing), and `None` is CPython's own
//! `Py_None` ([`foreign_pack::none_pointer`]).
//!
//! Ownership follows `boolop`'s boxing (Part 6 of #1371): the packer
//! borrows the native value, so an `int` temporary is released right after
//! it and nothing is retained. The packer's new reference belongs to
//! whatever slot receives it: a module-global `object` slot owns it and
//! releases it on rebind (Part 1 of #1499, `object_slot.rs`, which treats a
//! boxed `None` -- CPython's borrowed `Py_None` -- as borrowed and retains
//! it), and every other destination still never releases it (#1092's
//! leak-only rule, `docs/RUNTIME.md`). A packer `NULL` --
//! `OverflowError` for a bigint outside D-141's inline range (#1040) --
//! takes the foreign failure edge at once ([`foreign_fail::route_null`]).

use super::*;
use crate::foreign_fail::{ForeignFailEdge, route_null};
use crate::foreign_pack::{emit_pack, none_pointer};

/// Whether `value` moved into a slot of `slot_ty` is boxed: the slot is
/// `object` and the value is not.
///
/// The slot is tested first, so `value.ty()` -- which panics on malformed
/// hand-built MIR that `emit_expr` reports with its own message -- is
/// asked only of a value bound for an `object` slot.
pub(crate) fn boxes_into(value: &MirExpr, slot_ty: &pycc_mir::Ty) -> bool {
    *slot_ty == pycc_mir::Ty::Object && value.ty() != pycc_mir::Ty::Object
}

/// `value` evaluated and boxed into a new CPython object reference -- the
/// pointer a [`Scalar::Object`] carries.
///
/// A `None`-typed value is still evaluated for its effects (a call to a
/// `-> None` function), unless it is the literal itself, whose evaluation
/// has none.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_boxed<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
) -> PointerValue<'ctx> {
    // Non-strict mode (#3): a native display becomes the CPython container.
    if let Some(display) = as_object_display(value) {
        let built = emit_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            &display,
        );
        return crate::foreign_attr::expect_object_pointer(built);
    }
    if value.ty() == pycc_mir::Ty::None {
        if !matches!(value, MirExpr::NoneLiteral) {
            emit_expr(context, builder, module, rt, user_functions, locals, value);
        }
        return none_pointer(context, builder, module);
    }
    let scalar = crate::object_unbox::emit_pack_operand(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        value,
    );
    let packed = emit_pack(context, builder, module, scalar, "object_box");
    release_scalar_if_temporary(context, builder, rt, value, &scalar);
    route_null(
        context,
        builder,
        module,
        rt,
        ForeignFailEdge::for_current(builder),
        packed,
        "object_box",
    );
    packed
}

/// `value` rebuilt as the CPython container when it is a native list, tuple,
/// set or dict display (`pycc_types::object_box::display_boxable` admitted
/// it), nested displays included; `None` for every other value.
fn as_object_display(value: &MirExpr) -> Option<MirExpr> {
    let element = |e: &MirExpr| as_object_display(e).unwrap_or_else(|| e.clone());
    let display = |kind, elements: Vec<MirExpr>| MirExpr::ObjDisplay { kind, elements };
    Some(match value {
        MirExpr::ListLiteral(es) => MirExpr::ObjList {
            elements: es.iter().map(element).collect(),
        },
        MirExpr::TupleLiteral(es) => display(
            pycc_mir::ObjDisplayKind::Tuple,
            es.iter().map(element).collect(),
        ),
        MirExpr::SetLiteral { elements, .. } => display(
            pycc_mir::ObjDisplayKind::Set,
            elements.iter().map(element).collect(),
        ),
        MirExpr::DictLiteral(pairs) => display(
            pycc_mir::ObjDisplayKind::Dict,
            pairs
                .iter()
                .flat_map(|(k, v)| [element(k), element(v)])
                .collect(),
        ),
        _ => return None,
    })
}
