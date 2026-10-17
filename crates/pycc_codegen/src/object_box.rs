//! A native value boxed into an `object` slot (Part 2 of #1387, D-258's
//! boxing amendment).
//!
//! `pycc_types::object_box::admits` lets a native `int`, `float`, `bool`,
//! `str`, compiled-instance or `None` value (or a generator of such values,
//! `generator_box`) into an `object` slot at four
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
//! whatever slot or parameter receives it (`heap_rc`). A boxed `None` is
//! CPython's immortal `Py_None`, whose count does not matter. A packer
//! `NULL` --
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
    if let pycc_mir::Ty::Generator(sig) = value.ty() {
        return crate::generator_box::emit_boxed_generator(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            value,
            &sig.0,
        );
    }
    // Non-strict mode: a function value of object parameters and result
    // crosses as a CPython callable over it (`pycc_ext_obj_pack_closure`).
    if let pycc_mir::Ty::Callable(sig) = value.ty() {
        if let MirExpr::MakeClosure {
            function,
            captures,
            ..
        } = value
            && captures.is_empty()
        {
            return emit_boxed_function(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                value,
                function,
                sig.0.len(),
            );
        }
        let Scalar::Closure(closure) =
            emit_expr(context, builder, module, rt, user_functions, locals, value)
        else {
            panic!("pycc_codegen: internal error: a function value is not a closure")
        };
        let ptr = context.ptr_type(inkwell::AddressSpace::default());
        let i64_type = context.i64_type();
        let arity = i64_type.const_int(sig.0.len() as u64, false);
        let pack = crate::foreign_pack::shim_fn(
            module,
            crate::EXT_OBJ_PACK_CLOSURE_SYMBOL,
            ptr.fn_type(&[ptr.into(), i64_type.into()], false),
        );
        let packed = builder
            .build_call(pack, &[closure.into(), arity.into()], "pack_closure")
            .expect("build_call should not fail for the closure packer")
            .try_as_basic_value()
            .expect_basic("the closure packer returns a pointer")
            .into_pointer_value();
        if !crate::heap_rc::is_borrowed_read(value) {
            crate::heap_rc::emit_decref(context, builder, rt, crate::heap_rc::HeapKind::Closure, closure);
        }
        route_null(
            context,
            builder,
            module,
            rt,
            ForeignFailEdge::for_current(builder),
            packed,
            "object_box",
        );
        return packed;
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
        MirExpr::EmptyList(_) => MirExpr::ObjList {
            elements: Vec::new(),
        },
        MirExpr::EmptySet(_) => display(pycc_mir::ObjDisplayKind::Set, Vec::new()),
        MirExpr::EmptyDict(_) => display(pycc_mir::ObjDisplayKind::Dict, Vec::new()),
        MirExpr::TupleLiteral(es) => display(
            pycc_mir::ObjDisplayKind::Tuple,
            es.iter().map(element).collect(),
        ),
        MirExpr::SetLiteral { elements, .. } => display(
            if elements.len() >= 3
                && elements
                    .iter()
                    .all(crate::expr::container_expr::is_folded_constant)
            {
                pycc_mir::ObjDisplayKind::ConstantSet
            } else {
                pycc_mir::ObjDisplayKind::Set
            },
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

/// A module-level function `function` used as a value (`value`, a capture-free
/// `MirExpr::MakeClosure` of `arity` parameters), boxed into its CPython
/// callable: packed once, into a slot of its own
/// (`EXT_OBJ_PACK_FUNCTION_SYMBOL`), and read from that slot (with a new
/// reference) at every later evaluation, without building the native
/// closure again.
#[allow(clippy::too_many_arguments)]
fn emit_boxed_function<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
    function: &str,
    arity: usize,
) -> PointerValue<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();
    let symbol = format!("pycc.boxed_function.{function}");
    let slot = module.get_global(&symbol).unwrap_or_else(|| {
        let global = module.add_global(ptr, None, &symbol);
        global.set_initializer(&ptr.const_null());
        global.set_linkage(inkwell::module::Linkage::Private);
        global
    });
    let current = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("inside a function");
    let cached_bb = context.append_basic_block(current, "boxed_fn_cached");
    let first_bb = context.append_basic_block(current, "boxed_fn_first");
    let join_bb = context.append_basic_block(current, "boxed_fn_join");
    let cached = builder
        .build_load(ptr, slot.as_pointer_value(), "boxed_fn")
        .expect("load of the function's slot")
        .into_pointer_value();
    let is_cached = builder
        .build_is_not_null(cached, "boxed_fn_ready")
        .expect("null test of a pointer");
    crate::int_fast::likely_branch(context, builder, is_cached, cached_bb, first_bb);
    builder.position_at_end(cached_bb);
    builder
        .build_call(rt.obj_retain, &[cached.into()], "")
        .expect("build_call should not fail for the object retain");
    builder.build_unconditional_branch(join_bb).expect("branch");
    builder.position_at_end(first_bb);
    let Scalar::Closure(closure) = emit_expr(context, builder, module, rt, user_functions, locals, value) else {
        panic!("pycc_codegen: internal error: a function value is not a closure")
    };
    let pack = crate::foreign_pack::shim_fn(
        module,
        crate::EXT_OBJ_PACK_FUNCTION_SYMBOL,
        ptr.fn_type(&[ptr.into(), ptr.into(), i64_type.into()], false),
    );
    let packed = builder
        .build_call(
            pack,
            &[
                slot.as_pointer_value().into(),
                closure.into(),
                i64_type.const_int(arity as u64, false).into(),
            ],
            "pack_function",
        )
        .expect("build_call should not fail for the function packer")
        .try_as_basic_value()
        .expect_basic("the function packer returns a pointer")
        .into_pointer_value();
    crate::heap_rc::emit_decref(context, builder, rt, crate::heap_rc::HeapKind::Closure, closure);
    route_null(
        context,
        builder,
        module,
        rt,
        ForeignFailEdge::for_current(builder),
        packed,
        "object_box",
    );
    let first_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join_bb).expect("branch");
    builder.position_at_end(join_bb);
    let phi = builder.build_phi(ptr, "boxed_fn_value").expect("phi");
    phi.add_incoming(&[(&cached, cached_bb), (&packed, first_end)]);
    phi.as_basic_value().into_pointer_value()
}
