//! Multi-value twins (`pycc_mir::multi_return`): `MirStmt::SetOut`, which
//! stores a twin's result through its out-pointer, and
//! `MirStmt::AssignMulti`, which calls a twin and binds every result.
//!
//! A twin is declared with one trailing pointer parameter per result after
//! the first (`compile.rs`), bound in its frame under [`out_key`]. Each
//! result crosses as a reference the receiving temporary owns: `SetOut`
//! takes one for a borrowed value, and the caller stores the loaded values
//! into its temporaries without taking another. The twin stores its
//! results only on its way out of a normal return, so a call that raises
//! leaves the caller's out slots untouched and its temporaries unbound.

use super::*;

/// The frame key of a twin's out-pointer for result `index` (from 1).
pub(crate) fn out_key(index: usize) -> String {
    format!("0out_{index}")
}

/// `SetOut { index, value }`: `value`, as a reference of its own, through
/// out-pointer `index`.
pub(crate) fn emit_set_out_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::SetOut { index, value } = stmt else {
        unreachable!()
    };
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let scalar = heap_rc::retain_if_heap_duplicate(context, builder, rt, value, scalar);
    let scalar = retain_if_int_duplicate(context, builder, rt, value, scalar);
    let slot = locals
        .get(&out_key(*index))
        .expect("a twin's frame binds every out-pointer its body stores through");
    builder
        .build_store(slot.ptr, basic_value(scalar))
        .expect("build_store should not fail through a twin's out-pointer");
    Ok(())
}

/// `AssignMulti`: calls the twin and binds each temporary to its result,
/// or emits the fallback when the original's name may be bound to
/// something else at run time.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_assign_multi_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    let MirStmt::AssignMulti(multi) = stmt else {
        unreachable!()
    };
    let resolved = user_functions
        .get(multi.original.as_str())
        .filter(|original| original.direct_value.is_some() || original.single_def.is_some())
        .zip(user_functions.get(multi.twin.as_str()));
    let Some((original, twin)) = resolved else {
        return emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            &multi.fallback,
            expected_return_ty,
            finally_stack,
        );
    };
    // The original's name is looked up before any argument is evaluated,
    // as the call it replaces looks it up (a `NameError` before its `def`
    // has run); the twin itself is then called directly.
    let _ = user_call::resolve_callee(context, builder, rt, original);
    let twin_fn = twin
        .direct_value
        .expect("a multi-value twin is compiler-generated and dispatched directly");
    let outs: Vec<(PointerValue<'ctx>, &pycc_mir::Ty)> = multi.temps[1..]
        .iter()
        .map(|(_, ty)| {
            let basic = ty_to_basic_type(context, ty.clone());
            let slot = entry_slots::alloca_at_entry(builder, basic, "multi_out");
            builder
                .build_store(slot, default_value_for_type(context, ty.clone()))
                .expect("build_store should not fail for an out slot");
            (slot, ty)
        })
        .collect();
    let mut args = user_call::marshal_args(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &twin.param_tys,
        &multi.twin,
        &multi.args,
    );
    args.extend(outs.iter().map(|(slot, _)| inkwell::values::BasicMetadataValueEnum::from(*slot)));
    let call = builder
        .build_call(twin_fn, &args, "multi_call")
        .expect("build_call should not fail for a twin call");
    let first = crate::call_result::call_result_scalar(context, call, &multi.temps[0].1);
    guard_statement_effects(context, builder, rt);
    let mut results = vec![first];
    for (slot, ty) in &outs {
        let loaded = builder
            .build_load(ty_to_basic_type(context, (*ty).clone()), *slot, "multi_result")
            .expect("build_load should not fail for an out slot");
        results.push(scalar_of(ty, loaded));
    }
    for ((temp, _), result) in multi.temps.iter().zip(results) {
        let old = locals
            .get(temp)
            .and_then(|slot| heap_rc::take_slot_old(context, builder, rt, &slot.ty, slot.ptr));
        emit_assign(context, builder, rt, locals, temp, result);
        heap_rc::release_old(context, builder, rt, old);
    }
    Ok(())
}

/// `target = temp` followed by `ReleaseName(temp)`, for a compiler
/// temporary `temp` (a `0`-prefixed name) holding an `object` or a `str`
/// of the same type as `target`'s slot: the temporary's reference moves to
/// `target` and the temporary is left null, with no count traffic. `false`
/// (nothing emitted) for any other pair.
pub(crate) fn emit_move<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
    next: &MirStmt,
) -> bool {
    let (
        MirStmt::Assign {
            target,
            value: MirExpr::Name { name: temp, .. },
        },
        MirStmt::ReleaseName(released),
    ) = (stmt, next)
    else {
        return false;
    };
    if released != temp || !temp.starts_with('0') || target == temp {
        return false;
    }
    let (Some(source), Some(destination)) = (locals.get(temp), locals.get(target)) else {
        return false;
    };
    let counted = matches!(
        source.ty,
        pycc_mir::Ty::Object | pycc_mir::Ty::Str | pycc_mir::Ty::VarTuple(_)
    );
    if source.ty != destination.ty || !counted || rt.borrowed_locals.borrow().contains(target) {
        return false;
    }
    let pointer = context.ptr_type(inkwell::AddressSpace::default());
    let value = builder
        .build_load(pointer, source.ptr, "moved")
        .expect("build_load should not fail for a temporary's slot")
        .into_pointer_value();
    let scalar = match source.ty {
        pycc_mir::Ty::Str => Scalar::Str(value),
        pycc_mir::Ty::VarTuple(_) => Scalar::List(value),
        _ => Scalar::Object(value),
    };
    let old = heap_rc::take_slot_old(context, builder, rt, &destination.ty, destination.ptr);
    emit_assign(context, builder, rt, locals, target, scalar);
    heap_rc::release_old(context, builder, rt, old);
    builder
        .build_store(source.ptr, pointer.const_null())
        .expect("build_store should not fail for a temporary's slot");
    true
}

/// The LLVM value of a twin result.
fn basic_value(scalar: Scalar<'_>) -> inkwell::values::BasicValueEnum<'_> {
    match scalar {
        Scalar::Int(value) | Scalar::Bool(value) => value.into(),
        Scalar::Float(value) => value.into(),
        // A `tuple[T, ...]` result (v0.5) is its tuple object's pointer.
        Scalar::Str(value) | Scalar::Object(value) | Scalar::List(value) => value.into(),
        _ => panic!(
            "pycc_codegen: internal error: a multi-value twin result is not an int, float, \
             bool, str or object -- pycc_mir::multi_return admits no other"
        ),
    }
}

/// The scalar of type `ty` a loaded twin result is.
fn scalar_of<'ctx>(
    ty: &pycc_mir::Ty,
    value: inkwell::values::BasicValueEnum<'ctx>,
) -> Scalar<'ctx> {
    match ty {
        pycc_mir::Ty::Int => Scalar::Int(value.into_int_value()),
        pycc_mir::Ty::Bool => Scalar::Bool(value.into_int_value()),
        pycc_mir::Ty::Float => Scalar::Float(value.into_float_value()),
        pycc_mir::Ty::Str => Scalar::Str(value.into_pointer_value()),
        pycc_mir::Ty::Object => Scalar::Object(value.into_pointer_value()),
        pycc_mir::Ty::VarTuple(_) => Scalar::List(value.into_pointer_value()),
        other => panic!(
            "pycc_codegen: internal error: a multi-value twin result of type {other:?} -- \
             pycc_mir::multi_return admits only int, float, bool, str and object"
        ),
    }
}
