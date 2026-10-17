//! `b.extend([e for v in xs])` on a byte buffer, with the list built into
//! one list per site, reused by every run of the site: the comprehension's
//! list is only read by the extend (`pycc_ext_obj_ba_extend` copies its
//! bytes into the `bytearray`), so no run can observe that the list is the
//! previous run's, emptied (`pycc_rt_list_reset`) -- not a new one. A
//! sub-byte row unpacker (`out.extend([mask & (o >> i) for i in shifts])`
//! per byte) no longer allocates and frees a list per byte.
//!
//! Only a comprehension whose source, filter and element run no code that
//! could reach the same site again is built this way: names, literals,
//! `int` arithmetic and comparisons, conditional expressions and byte
//! buffer items (`reentrancy_free`). The list stays in its site's global
//! for the module's lifetime; an exception while it is filled leaves it
//! partly filled, and the next run empties it first.

use super::*;
use pycc_mir::{CompSource, MirCompElt, RtCallExpr};

const EXTEND: &str = "pycc_ext_obj_ba_extend";

/// Whether evaluating `expr` runs no code that could reach the same site
/// (module doc).
fn reentrancy_free(expr: &MirExpr) -> bool {
    let shallow = match expr {
        MirExpr::IntLiteral(_)
        | MirExpr::BoolLiteral(_)
        | MirExpr::Name { .. }
        | MirExpr::BinOp { .. }
        | MirExpr::Compare { .. }
        | MirExpr::IfExp { .. }
        | MirExpr::Not(_) => true,
        MirExpr::Nd(nd) => {
            matches!(
                nd.op,
                pycc_hir::ndarray::NdOp::Get { .. } | pycc_hir::ndarray::NdOp::Dim(_)
            )
        }
        _ => false,
    };
    shallow && expr.children().into_iter().all(reentrancy_free)
}

/// Emits `call` when it is such an extend (module doc); `None` otherwise.
pub(super) fn emit<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    call: &RtCallExpr,
) -> Option<Scalar<'ctx>> {
    let [buffer, MirExpr::Comprehension(comp)] = call.args.as_slice() else {
        return None;
    };
    let MirCompElt::List(elt) = &comp.elt else {
        return None;
    };
    let source_ok = match &comp.source {
        CompSource::List(_) => true,
        CompSource::Range { start, stop, step } => {
            [start, stop, step].into_iter().all(reentrancy_free)
        }
        _ => false,
    };
    if call.sig.symbol != EXTEND
        || comp.collects_objects()
        || comp.var_ty != pycc_mir::Ty::Int
        || !source_ok
        || !reentrancy_free(elt)
        || !comp.cond.as_ref().is_none_or(reentrancy_free)
    {
        return None;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    // The buffer first, as the call evaluates its arguments.
    let buffer_scalar = emit_expr(context, builder, module, rt, user_functions, locals, buffer);
    let held = crate::object_release::hold(context, module, rt, buffer, &buffer_scalar);
    let Scalar::Object(buffer_ptr) = buffer_scalar else {
        panic!("pycc_codegen: internal error: a byte buffer is not an object")
    };
    let mut n = 0;
    while module
        .get_global(&format!("pycc.extend_scratch.{n}"))
        .is_some()
    {
        n += 1;
    }
    let slot = module.add_global(ptr, None, &format!("pycc.extend_scratch.{n}"));
    slot.set_initializer(&ptr.const_null());
    slot.set_linkage(inkwell::module::Linkage::Private);
    let function = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("inside a function");
    let new_bb = context.append_basic_block(function, "extend_scratch_new");
    let reset_bb = context.append_basic_block(function, "extend_scratch_reset");
    let ready_bb = context.append_basic_block(function, "extend_scratch_ready");
    let cached = builder
        .build_load(ptr, slot.as_pointer_value(), "extend_scratch")
        .expect("load of the site's list")
        .into_pointer_value();
    let present = builder
        .build_is_not_null(cached, "extend_scratch_present")
        .expect("null test");
    crate::int_fast::likely_branch(context, builder, present, reset_bb, new_bb);
    builder.position_at_end(reset_bb);
    builder
        .build_call(rt.list_reset, &[cached.into()], "")
        .expect("build_call should not fail for pycc_rt_list_reset");
    builder
        .build_unconditional_branch(ready_bb)
        .expect("branch");
    builder.position_at_end(new_bb);
    let made = list::build_list_new(context, builder, rt, &elt.ty(), 0);
    builder
        .build_store(slot.as_pointer_value(), made)
        .expect("store of the site's list");
    builder
        .build_unconditional_branch(ready_bb)
        .expect("branch");
    builder.position_at_end(ready_bb);
    let phi = builder.build_phi(ptr, "extend_scratch_list").expect("phi");
    phi.add_incoming(&[(&cached, reset_bb), (&made, new_bb)]);
    let list_ptr = phi.as_basic_value().into_pointer_value();
    rt.comprehension_into.set(Some(list_ptr));
    let filled = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &call.args[1],
    );
    rt.comprehension_into.set(None);
    let Scalar::List(filled) = filled else {
        panic!("pycc_codegen: internal error: a list comprehension is not a list")
    };
    let extend = module.get_function(EXTEND).unwrap_or_else(|| {
        module.add_function(
            EXTEND,
            ptr.fn_type(&[ptr.into(), ptr.into()], false),
            Some(inkwell::module::Linkage::External),
        )
    });
    let result = builder
        .build_call(extend, &[buffer_ptr.into(), filled.into()], "rt_call")
        .expect("build_call should not fail for the byte buffer extend");
    held.release(builder, rt);
    Some(crate::call_result::call_result_scalar(
        context,
        result,
        &call.sig.ret,
    ))
}
