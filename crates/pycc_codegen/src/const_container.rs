//! A constant container a membership test reads, built once (non-strict
//! mode): `kind not in (1, 2, 3, 4)` with a CPython object `kind` boxes the
//! display into a CPython tuple at every evaluation otherwise.
//!
//! The display is a tuple, list or set of literals (`int`s in the smallint
//! range, `str`, `float`, `bool`, `None`); CPython folds the set into a
//! frozenset constant. The membership test only reads it --
//! `PySequence_Contains` compares each element with the item, and nothing
//! can reach the container itself -- and its elements are immutable, so one
//! object built at the site's first evaluation answers every later test the
//! same way (CPython shares the constant tuple of `x in (1, 2)` likewise).
//! The object is kept in a per-site global for the module's lifetime and
//! read borrowed.

use super::*;
use crate::foreign_fail::{ForeignFailEdge, route_null};

/// Whether `container` (an `ObjContains` container) is such a display.
pub(crate) fn is_constant_sequence(container: &MirExpr) -> bool {
    let elements = match container {
        MirExpr::ObjectBox(inner) => match inner.as_ref() {
            MirExpr::TupleLiteral(es) | MirExpr::ListLiteral(es) => es,
            // A set display of literals (`kind in {"a", "b"}`), which
            // CPython folds into a frozenset constant: the set built once
            // is only read by the membership test, as that constant is.
            MirExpr::SetLiteral { elements, .. } => elements,
            _ => return false,
        },
        MirExpr::ObjDisplay {
            kind: pycc_mir::ObjDisplayKind::Set | pycc_mir::ObjDisplayKind::ConstantSet,
            elements,
        } => elements,
        _ => return false,
    };
    !elements.is_empty()
        && elements.iter().all(|e| {
            let e = match e {
                MirExpr::ObjectBox(inner) => inner.as_ref(),
                other => other,
            };
            match e {
                MirExpr::IntLiteral(n) => crate::int_const::fits_tagged_smallint(*n).is_some(),
                MirExpr::StringLiteral(_)
                | MirExpr::FloatLiteral(_)
                | MirExpr::BoolLiteral(_)
                | MirExpr::NoneLiteral => true,
                _ => false,
            }
        })
}

/// The site's CPython object for `container` (an [`is_constant_sequence`]
/// display), borrowed: built and stored on the first evaluation.
pub(crate) fn emit_cached_constant<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    container: &MirExpr,
) -> inkwell::values::PointerValue<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let mut n = 0;
    while module
        .get_global(&format!("pycc.const_container.{n}"))
        .is_some()
    {
        n += 1;
    }
    let slot = module.add_global(ptr, None, &format!("pycc.const_container.{n}"));
    slot.set_initializer(&ptr.const_null());
    slot.set_linkage(inkwell::module::Linkage::Private);
    let function = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("inside a function");
    let first_bb = context.append_basic_block(function, "const_container_first");
    let join_bb = context.append_basic_block(function, "const_container_join");
    let entry_bb = builder.get_insert_block().expect("positioned");
    let cached = builder
        .build_load(ptr, slot.as_pointer_value(), "const_container")
        .expect("load of the site's slot")
        .into_pointer_value();
    let ready = builder
        .build_is_not_null(cached, "const_container_ready")
        .expect("null test of a pointer");
    crate::int_fast::likely_branch(context, builder, ready, join_bb, first_bb);
    builder.position_at_end(first_bb);
    let Scalar::Object(built) = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        container,
    ) else {
        panic!("pycc_codegen: internal error: a boxed display is not an object")
    };
    route_null(
        context,
        builder,
        module,
        rt,
        ForeignFailEdge::for_current(builder),
        built,
        "const_container",
    );
    // The slot keeps the new reference for good.
    builder
        .build_store(slot.as_pointer_value(), built)
        .expect("store into the site's slot");
    let first_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join_bb).expect("branch");
    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(ptr, "const_container_value")
        .expect("phi");
    phi.add_incoming(&[(&cached, entry_bb), (&built, first_end)]);
    phi.as_basic_value().into_pointer_value()
}
