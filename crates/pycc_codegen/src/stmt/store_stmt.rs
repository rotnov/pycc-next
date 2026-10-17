//! Assignment, element/attribute stores and expression statements.

use super::*;

// A user-function call whose declared return type is `None`, used
// as a bare statement (e.g. `main()`) -- must go through
// `build_call_to` directly rather than the general `ExprStmt(expr)`
// arm below: the call itself returns LLVM `void`, and the general
// expression path's canonical unit carrier is unnecessary when the
// result is discarded. Matched by `ty`
// alone (no `callee != "print"` guard needed): the arm above
// already claims every `print` call via its own guard regardless
// of `ty`, so only a non-`print` call ever reaches this one.
//
// Generalizes this crate's pre-Task-5 zero-arg-only special case
// (which this task's brief called "redundant" and removed) to a
// call of any arity: unlike `emit_expr`'s `Call` arm, this arm
// *does* still have a `Result` to propagate a clean, user-facing
// error through for an undefined callee, exactly like the
// zero-arg case always did -- so it keeps doing that, rather than
// switching to `emit_expr`'s "internal error" panic, preserving
// both the existing user-facing behavior and the `?`-propagation
// coverage every nested-body call site below relies on.
pub(crate) fn emit_expr_stmt_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::ExprStmt(MirExpr::Call {
        callee,
        args,
        ty: pycc_mir::Ty::None,
    }) = stmt
    else {
        unreachable!()
    };
    let user_function = user_functions
        .get(callee.as_str())
        .ok_or_else(|| format!("pycc_codegen v0.1: call to undefined function `{callee}`"))?;
    let _ = user_function; // validated above; build_call_to looks up by name
    build_call_to(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        callee,
        args,
    );
    guard_statement_effects(context, builder, rt);
    Ok(())
}

/// `x = x op y` (`x += y`) for an `int` function local `x`, `op` one of
/// `+ - *` and `y` a name, a literal or an `int64` array element read
/// with a name for the array and simple indices: the old value of `x` is
/// the left operand itself, so it is handed to the operator as an owned operand,
/// which releases it only on the runtime path. On the smallint fast path
/// it is a smallint and owns nothing, so the store needs no release test.
/// Returns `false` (emitting nothing) for any other assignment.
#[allow(clippy::too_many_arguments)]
fn emit_int_self_update<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    target: &str,
    value: &MirExpr,
) -> bool {
    let MirExpr::BinOp {
        op,
        left,
        right,
        ty: pycc_mir::Ty::Int,
    } = value
    else {
        return false;
    };
    let is_target = |expr: &MirExpr| {
        matches!(expr, MirExpr::Name { name, ty: pycc_mir::Ty::Int } if name == target)
    };
    let simple = |expr: &MirExpr| {
        matches!(
            expr,
            MirExpr::Name {
                ty: pycc_mir::Ty::Int,
                ..
            } | MirExpr::IntLiteral(_)
        )
    };
    let Some(slot) = locals.get(target) else {
        return false;
    };
    // Proven smallints compute without a fast-path test at all
    // (`int_small`, through the ordinary assignment).
    if crate::int_small::binop_operands(&rt.int_ranges.borrow(), *op, left, right, &pycc_mir::Ty::Int)
        .is_some()
    {
        return false;
    }
    let is_alloca = slot
        .ptr
        .as_instruction()
        .is_some_and(|inst| inst.get_opcode() == inkwell::values::InstructionOpcode::Alloca);
    // `x += a[i]` on an `int64` array: reading the element runs no code
    // that could rebind `x` (`a` is a name, the indices names, literals or
    // their sum or difference), and the element is a fresh word.
    let element = match right.as_ref() {
        MirExpr::Nd(nd)
            if crate::ndarray::is_i64_element(nd)
                && matches!(nd.base, MirExpr::Name { .. })
                && nd.args.iter().all(crate::ndarray::is_simple_index) =>
        {
            Some(nd)
        }
        _ => None,
    };
    if !matches!(
        op,
        pycc_mir::BinOpKind::Add | pycc_mir::BinOpKind::Sub | pycc_mir::BinOpKind::Mul
    ) || !is_target(left)
        || !(simple(right) || element.is_some())
        || slot.ty != pycc_mir::Ty::Int
        || !is_alloca
        || rt.borrowed_locals.borrow().contains(target)
    {
        return false;
    }
    let slot = slot.clone();
    let l = to_numeric_encoded_int(
        context,
        builder,
        emit_expr(context, builder, module, rt, user_functions, locals, left),
    );
    let result = match element {
        // The element joins the arithmetic as a machine integer: no `int`
        // is made of it unless the sum leaves the inline range.
        Some(nd) if *op != pycc_mir::BinOpKind::Mul => {
            let raw = crate::ndarray::emit_nd_raw_i64(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                nd,
            );
            crate::int_fast::emit_int_addsub_i64(context, builder, rt, *op, l, raw, &[l])
        }
        _ => {
            let r = to_numeric_encoded_int(
                context,
                builder,
                emit_expr(context, builder, module, rt, user_functions, locals, right),
            );
            // An element read is a fresh word, owned like the old value.
            let owned: &[_] = if element.is_some() { &[l, r] } else { &[l] };
            crate::int_fast::emit_int_binop(context, builder, rt, *op, l, r, owned)
        }
    };
    crate::entry_slots::store_unowned_int(context, builder, &slot, result);
    true
}

pub(crate) fn emit_assign_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::Assign { target, value } = stmt else {
        unreachable!()
    };
    if emit_int_self_update(context, builder, module, rt, user_functions, locals, target, value) {
        return Ok(());
    }
    // Non-strict mode: an object rebinding an `int`, `float` or `bool`
    // parameter is converted with a run-time type check
    // (`pycc_types::non_strict::admits_rebinding_coercion`). The converted
    // value is fresh, so it is stored without a retain.
    if let Some(slot_ty) = locals.get(target).map(|slot| slot.ty.clone())
        && matches!(slot_ty, pycc_mir::Ty::Int | pycc_mir::Ty::Float | pycc_mir::Ty::Bool)
        && crate::object_coerce::coerces_into(value, &slot_ty)
    {
        let scalar = crate::object_coerce::emit_coerced(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            value,
            &slot_ty,
            &format!("the value assigned to `{target}`"),
        );
        emit_assign(context, builder, rt, locals, target, scalar);
        return Ok(());
    }
    let ty = value.ty();
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    // The slot owns one reference to a counted value: take it, and
    // release the one it holds (null before the first store) once
    // the new one is stored -- unless the local only ever borrows.
    if ty == pycc_mir::Ty::Str
        && locals
            .get(target)
            .is_some_and(|slot| slot.ty != pycc_mir::Ty::Str)
    {
        panic!(
            "pycc_codegen: internal error: string assignment target `{target}` has a \
             non-string storage slot"
        );
    }
    let owning = !rt.borrowed_locals.borrow().contains(target);
    let scalar = if owning {
        heap_rc::retain_if_heap_duplicate(context, builder, rt, value, scalar)
    } else {
        scalar
    };
    let scalar = retain_if_int_duplicate(context, builder, rt, value, scalar);
    let old = locals
        .get(target)
        .filter(|_| owning)
        .and_then(|slot| heap_rc::take_slot_old(context, builder, rt, &slot.ty, slot.ptr));
    // Part 2a of #1142 (#1165): D-074's free-before-overwrite, for
    // the second owning slot type. Keyed on `ty` exactly like the
    // `str` arm above; unlike `int`, a buffer slot's declared type
    // and the value's type can never disagree, because the producer
    // is the only expression of the type a local can be bound to.
    if ty == pycc_mir::Ty::MemoryView {
        free_buffer_slot_before_store(context, builder, rt, locals, target);
    }
    // #146 Part 1: the matching `int` release is *not* here. It
    // lives inside `emit_assign`, gated on the target slot's own
    // declared type rather than on `ty` above -- `x: int = True`
    // reaches this arm with `ty == Ty::Bool` and would otherwise
    // skip the release of the bigint the slot still holds.
    emit_assign(context, builder, rt, locals, target, scalar);
    heap_rc::release_old(context, builder, rt, old);
    Ok(())
}

// `b[i] = v` on a `memoryview` parameter (Part 1 of #1142): the
// store counterpart of `MirExpr::BufferGet` above, sharing its
// base extraction and its index decoding verbatim so the load and
// the store can never disagree about either.
//
// The `guard_statement_effects` call is load-bearing and is
// deliberately *not* copied from `MirStmt::DictSet` below, which
// has none: `pycc_rt_dict_set` cannot raise, while
// `pycc_rt_buffer_f64_set` raises `IndexError` exactly as the load
// does. The load lets `expression_can_set_exception` guard the
// expression that consumes its `0.0` sentinel; a `void` setter has
// no consumer, so the statement-level guard is the *only* thing
// that ever observes the raise. Omit it and an out-of-range store
// as a function's last statement would return normally. The
// precedent copied here is `MirStmt::Call`'s own guard above.
//
// A *sub-expression* that raises is already handled before the
// write: `emit_expr` guards any operand for which
// `expression_can_set_exception` holds -- a `b[0] = b[99]` load
// among them -- and branches away before this arm reaches
// `buffer_f64_set`, so no write lands on a sentinel operand. A
// bigint index raises `IndexError` in `build_untag_checked`, which
// guards it the same way.
pub(crate) fn emit_buffer_set_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::BufferSet { base, index, value } = stmt else {
        unreachable!()
    };
    // CPython's assignment order, which is *not* left to right: for
    // `b[i] = v` the interpreter evaluates the right-hand side
    // first, then the subscription target, then the index. A
    // `d[i()] = v()` probe on CPython prints `v`'s effect before
    // `i`'s, so emitting the index first would let a raising or
    // side-effecting index expression suppress a value expression
    // CPython would already have run. The three `emit_expr` calls
    // below are therefore in value, base, index order.
    //
    // The neighbouring `MirStmt::DictSet` arm has the same ordering
    // defect -- it emits its key before its value. That is
    // pre-existing and out of scope here, and is worth its own issue
    // rather than a silent fix in this pull request, so it is left
    // exactly as it stands.
    let value_scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    // `pycc_types` admits only a `Ty::Float` value here, so this is
    // the identity arm of `to_float`; it is used rather than an
    // inline `Scalar::Float` destructure so the store carries no
    // panic the load does not.
    let element = to_float(context, builder, rt, value_scalar);
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let Scalar::MemoryView(base_ptr) = base_scalar else {
        panic!(
            "pycc_codegen: internal error: a buffer element store's base did not \
             evaluate to a memoryview"
        )
    };
    let index_scalar = emit_expr(context, builder, module, rt, user_functions, locals, index);
    let encoded_index = to_numeric_encoded_int(context, builder, index_scalar);
    let raw_index = build_untag_checked(
        context,
        builder,
        rt,
        UntagUse::Index,
        encoded_index,
        "buffer_untag_index",
    );
    builder
        .build_call(
            rt.buffer_f64_set,
            &[base_ptr.into(), raw_index.into(), element.into()],
            "buffer_set",
        )
        .expect("build_call should not fail for a declared runtime function");
    guard_statement_effects(context, builder, rt);
    Ok(())
}

// `xs[i] = v` on a list: CPython's value, base, index order (see
// `BufferSet` above); the list adopts the stored element's
// reference and an out-of-range index raises `IndexError`.
pub(crate) fn emit_list_set_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::ListSet { base, index, value } = stmt else {
        unreachable!()
    };
    let elem = list::element_ty(&base.ty()).clone();
    let value_scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let word = list::element_word(context, builder, rt, &elem, value, value_scalar);
    // The element (owned once `element_word` took it) and a fresh
    // target are released if the target or index raises; the store
    // itself releases the element on its own failure.
    let mark = heap_rc::pending_mark(rt);
    heap_rc::hold_owned_value(rt, &elem, &value_scalar);
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let list_ptr = expect_list_pointer(base_scalar, "the item-assignment target");
    heap_rc::hold_temporary(rt, base, &base_scalar);
    let index_scalar = emit_expr(context, builder, module, rt, user_functions, locals, index);
    let raw = list::raw_index(context, builder, rt, index_scalar);
    heap_rc::truncate_pending(rt, mark);
    heap_rc::hold_temporary(rt, base, &base_scalar);
    list::build_list_set(context, builder, rt, &elem, list_ptr, raw, word);
    heap_rc::truncate_pending(rt, mark);
    // A fresh target list is dead once stored into.
    release_scalar_if_temporary(context, builder, rt, base, &base_scalar);
    Ok(())
}

// `d[k] = v` (PR-11 Task 5, D-123): insert-or-update --
// `pycc_rt_dict_set` itself decides which, by whether `key`
// already compares equal to a stored key. `dict` is read by name
// (like a bare-name `MirExpr::ListAppend` receiver), `key` crosses
// in as a `Ty::Str` expression unchanged, exactly like
// `MirExpr::DictGet`'s key, and `value` gets D-141 validation plus
// identity-preserving encoded storage.
pub(crate) fn emit_dict_set_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::DictSet { dict, key, value } = stmt else {
        unreachable!()
    };
    let (key_ty, value_ty) = containers::dict_kv(&locals[dict].ty);
    // CPython evaluates the value, then the container and the key.
    let value_scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let value_scalar = coerce_scalar_to_type(context, builder, value_scalar, value_ty.clone());
    let value_word = list::element_word(context, builder, rt, &value_ty, value, value_scalar);
    let dict_ptr = emit_dict_name_read(context, builder, module, rt, user_functions, locals, dict);
    let key_scalar = emit_expr(context, builder, module, rt, user_functions, locals, key);
    let key_scalar = coerce_scalar_to_type(context, builder, key_scalar, key_ty.clone());
    let key_word = list::element_word(context, builder, rt, &key_ty, key, key_scalar);
    build_dict_set(builder, rt, dict_ptr, key_word, value_word);
    guard_statement_effects(context, builder, rt);
    Ok(())
}

// Part 2c of #1371: `del o[a:b:c]`, the statement twin of the
// `MirExpr::ObjSlice` arm, sharing its operand order and packing.
pub(crate) fn emit_obj_del_slice_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::ObjDelSlice {
        base,
        start,
        stop,
        step,
    } = stmt
    else {
        unreachable!()
    };
    foreign_slice::emit_with_operands(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        base,
        [start.as_ref(), stop.as_ref(), step.as_ref()],
        |base, bounds| foreign_slice::emit_del_slice(context, builder, module, rt, base, bounds),
    );
    Ok(())
}

// Non-strict mode (#3): `o[a:b:c] = v`, the store twin of `del o[a:b:c]`;
// `foreign_slice::emit_set_slice` owns the evaluation order.
pub(crate) fn emit_obj_set_slice_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::ObjSetSlice {
        base,
        start,
        stop,
        step,
        value,
    } = stmt
    else {
        unreachable!()
    };
    foreign_slice::emit_set_slice(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        base,
        [start.as_ref(), stop.as_ref(), step.as_ref()],
        value,
    );
    Ok(())
}

// #1457 (Part 2 of #1443): `o.x = v` and `del o.x` on a CPython
// object; `foreign_store` owns the evaluation order and packing.
pub(crate) fn emit_obj_attr_set_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::ObjAttrSet { base, attr, value } = stmt else {
        unreachable!()
    };
    foreign_store::emit_set_attr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        base,
        attr,
        value,
    );
    Ok(())
}

// Non-strict mode (#3): `o[k] = v` and `del o[k]` on a CPython object.
pub(crate) fn emit_obj_set_item_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::ObjSetItem { base, key, value } = stmt else {
        unreachable!()
    };
    foreign_arith::emit_set_item(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        base,
        key,
        value,
    );
    Ok(())
}

pub(crate) fn emit_obj_del_item_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::ObjDelItem { base, key } = stmt else {
        unreachable!()
    };
    foreign_arith::emit_del_item(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        base,
        key,
    );
    Ok(())
}

// `base.attr = value` (D-154, Part 1 of #375): stores the raw slot
// word inline (`attr_slot::emit_slot_store`) -- the
// mirror image of `MirExpr::AttrGet`'s read, using
// `scalar_to_slot_word` for the identical encoding
// `slot_word_to_scalar` decodes.
//
// For a `Ty::Str` attribute, mirrors `MirStmt::Assign`'s own two
// refcount obligations exactly (D-154 Part 1's own post-merge
// review finding -- the first version of this arm had neither):
// `incref_if_str_duplicate` before storing, since a source value
// `str_value_is_a_duplicate_reference` classifies as borrowed is a
// *duplicate* reference whose original owner keeps its own copy; and
// `decref_str_attr_slot_before_store` before overwriting, to
// release whatever the slot held previously (a no-op on a fresh
// instance's unassigned slot, which the unchecked getter reads as
// `0`, exactly like a local's
// null-initialized string slot). Without both, a `str` attribute
// read twice, or reassigned, use-after-frees the first `PyStrObj`.
pub(crate) fn emit_attr_set_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::AttrSet { base, slot, value } = stmt else {
        unreachable!()
    };
    // CPython's order: the value, then the target's base
    // (`b.p.name = f(b)` stores into the `b.p` that `f` left).
    let value_ty = value.ty();
    let value_scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let value_scalar = heap_rc::retain_if_heap_duplicate(context, builder, rt, value, value_scalar);
    let value_scalar = retain_if_int_duplicate(context, builder, rt, value, value_scalar);
    // The value is released if evaluating the base raises.
    let mark = heap_rc::pending_mark(rt);
    heap_rc::hold_owned_value(rt, &value_ty, &value_scalar);
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    heap_rc::truncate_pending(rt, mark);
    let base_ptr = expect_instance_pointer(base_scalar, "attribute assignment base");
    let slot = *slot as u64;
    // The slot owns a counted value: the one it holds is released
    // once the new one is stored (`heap_rc::take_slot_old`).
    let old = heap_rc::take_attr_slot_old(context, builder, rt, &value_ty, base_ptr, slot);
    // Unlike a local's storage slot, an instance attribute's
    // declared `Ty` is not reachable from `MirStmt::AttrSet` (it
    // carries only a slot *index*), so this release is gated on
    // the *value's* type rather than the slot's.
    //
    // #146 Part 1 recorded one shape that diverged: a `bool` value
    // assigned into an `int`-declared attribute reported
    // `Ty::Bool` here, skipping the release of a bigint the
    // attribute still held -- a leak, never a use-after-free
    // (D-180 Consequences item 6). #627 closed that divergence in
    // MIR instead of here: `pycc_mir`'s `HirStmt::AttrSet` arm now
    // wraps such a value in `MirExpr::IntBoundary`, whose `ty()`
    // is `Ty::Int`, so the release below fires and the store
    // encodes a D-141 word. `tests/issue_146_bigint_release.rs`
    // pins the observable half and `bigint_rc.rs`'s own
    // `an_int_attribute_slot_store_of_a_bool_emits_a_guarded_release`
    // pins this release. D-187 records the correction.
    //
    // The gate stays literally value-typed and only *coincides*
    // with D-180 Decision item 4's slot-typed invariant because
    // `bool` -> `int` is the sole widening `pycc_types` permits at
    // an attribute boundary (`bool` into a `float` attribute is
    // rejected with `error[T0021]`). A future widening would break
    // the coincidence silently; threading the declared type
    // through from `mir.class_defs` remains out of scope.
    if value_ty == pycc_mir::Ty::Int {
        release_int_attr_slot_before_store(context, builder, rt, base_ptr, slot);
    }
    let word = scalar_to_slot_word(context, builder, value_scalar);
    // A slot proven always assigned already has its flag set, except while
    // a constructor's prefix is setting it.
    let flag_set = !rt.emitting_ctor.get()
        && rt
            .assigned_slots
            .borrow()
            .is_always_assigned(&base.ty(), slot as usize);
    attr_slot::emit_slot_store(context, builder, base_ptr, slot, word, !flag_set);
    heap_rc::release_old(context, builder, rt, old);
    // A fresh target (`f().x = v`, or a base `operand_pin` pinned
    // across the value) is dead once stored into.
    release_scalar_if_temporary(context, builder, rt, base, &base_scalar);
    Ok(())
}
