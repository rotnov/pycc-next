//! Attribute reads, CPython `object` operations and `Optional` wrapping.

use super::*;

// CPython's `NotImplemented` singleton (#1418), a borrowed pointer
// from the fixed shim -- only an `ext` module's admitted
// `return NotImplemented` builds this node.
pub(crate) fn emit_not_implemented_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::NotImplemented = expr else {
        unreachable!()
    };
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let accessor = foreign_pack::shim_fn(
        module,
        EXT_OBJ_NOT_IMPLEMENTED_SYMBOL,
        ptr.fn_type(&[], false),
    );
    Scalar::Object(
        builder
            .build_call(accessor, &[], "not_implemented")
            .expect("build_call should not fail for pycc_ext_obj_not_implemented")
            .try_as_basic_value()
            .expect_basic("pycc_ext_obj_not_implemented returns PyObject *")
            .into_pointer_value(),
    )
}

// `OptionalWrap` (D-197, #763, Part 1 of #747) exists purely to fix
// `.ty()` for `collect_stmt_bindings`'s slot-type derivation (see
// its own doc comment in `pycc_mir`) -- the actual `{ inner, i8 }`
// struct-building work is `coerce_scalar_to_type`'s, called here
// with the wrapper's own declared `Ty::Optional(inner)` as the
// target so it applies uniformly whether the wrapped value is a
// bare `inner`-typed payload or `MirExpr::NoneLiteral`'s
// placeholder.
pub(crate) fn emit_optional_wrap_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::OptionalWrap(value, inner) = expr else {
        unreachable!()
    };
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    // #770 review: `value`'s classification must be checked here,
    // against the raw `Scalar` it just produced, *before*
    // `coerce_scalar_to_type` below wraps it into a
    // `Scalar::Optional` struct. `MirStmt::Assign`'s own
    // `retain_if_int_duplicate` call runs on the *outer*
    // `OptionalWrap` expression and its already-wrapped
    // `Scalar::Optional` result, which can never retain anything
    // (`retain_if_int_duplicate`'s `if let Scalar::Int(word) =
    // scalar` guard never matches a struct) and
    // `int_value_is_a_duplicate_reference` classifies
    // `OptionalWrap` itself as owning for exactly that reason --
    // by design, it never looks inside the wrapper. So a borrowed
    // `int` payload (e.g. `x: int | None = n` for a heap-bigint
    // `n`) reached this arm with no retain anywhere in the
    // pipeline before this fix, letting `x` hold a second,
    // unretained reference to `n`'s bigint: `n`'s later
    // reassignment/death still fires `release_int_slot_before_store`
    // unconditionally, freeing the object out from under `x`'s
    // stored payload despite `x` still being live. Reusing the
    // same classification here (rather than duplicating it) keeps
    // both the direct-assignment and the `OptionalWrap` paths
    // agreeing on which shapes are borrowed.
    let scalar = retain_if_int_duplicate(context, builder, rt, value, scalar);
    coerce_scalar_to_type(
        context,
        builder,
        scalar,
        pycc_mir::Ty::Optional(inner.clone()),
    )
}

pub(crate) fn emit_optional_unwrap_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::OptionalUnwrap(value, inner) = expr else {
        unreachable!()
    };
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let Scalar::Optional(v) = scalar else {
        panic!(
            "pycc_codegen: internal error: OptionalUnwrap's operand did not evaluate to Scalar::Optional -- pycc_mir should only ever wrap a Ty::Optional-scoped Name read"
        );
    };
    let payload = builder
        .build_extract_value(v, 0, "narrowed_payload")
        .expect("build_extract_value should not fail extracting field 0 of an Optional struct");
    // #809 (Part 3 of #747): `T0049`'s widened gate now admits
    // `Ty::Float`/`Ty::Bool` inner types alongside the pre-existing
    // `Ty::Int`, so the narrowed payload must become the matching
    // `Scalar` variant instead of always `Scalar::Int` -- a plain
    // `f64`/`i8` payload handed to an `int`-typed consumer as
    // `Scalar::Int` would misinterpret it as a D-141-encoded word.
    match inner.as_ref() {
        pycc_mir::Ty::Float => Scalar::Float(payload.into_float_value()),
        pycc_mir::Ty::Bool => Scalar::Bool(payload.into_int_value()),
        // `Ty::Int`, the pre-existing shape -- and every other
        // `Ty` is unreachable here: `T0049`
        // (`crates/pycc_hir/src/func.rs`) restricts every
        // `Ty::Optional` inner type to `{int, float, bool}` before
        // an `OptionalUnwrap` node can ever be constructed.
        _ => Scalar::Int(payload.into_int_value()),
    }
}

// `base.attr` (D-154): an inline read of the slot word, raising
// `AttributeError` on the cold path for a slot not yet assigned
// (#1388) -- see `attr_slot::emit_slot_load_checked` -- then
// reinterpreted as the attribute's own declared `Ty` (see
// `slot_word_to_scalar`).
pub(crate) fn emit_attr_get_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::AttrGet { base, slot, ty } = expr else {
        unreachable!()
    };
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let base_ptr = expect_instance_pointer(base_scalar, "attribute access base");
    // A fresh base is released if the slot is unassigned.
    let mark = heap_rc::pending_mark(rt);
    heap_rc::hold_temporary(rt, base, &base_scalar);
    let raw = if rt.assigned_slots.borrow().is_always_assigned(&base.ty(), *slot) {
        attr_slot::emit_slot_load(context, builder, base_ptr, *slot as u64)
    } else {
        attr_slot::emit_slot_load_checked(context, builder, rt, base_ptr, *slot as u64)
    };
    heap_rc::truncate_pending(rt, mark);
    let value = slot_word_to_scalar(context, builder, raw, ty);
    // A fresh base (`f().x`) dies right after the read, so the read
    // takes its own reference first (`heap_rc::is_borrowed_read`).
    if !heap_rc::is_borrowed_read(base) {
        heap_rc::retain_scalar(context, builder, rt, ty, &value);
        release_scalar_if_temporary(context, builder, rt, base, &base_scalar);
    }
    value
}

// Part 2 of #1026 (PR 2b of #1081): the call counterpart of
// `ObjAttrGet` directly above. The order below is CPython's own --
// base, then the callable, then each argument left to right -- and
// resolving the callable *here*, rather than inside the call shim,
// is the whole reason `foreign_call` exposes two functions:
// `obj.missing(1 // 0)` must raise `AttributeError`, not
// `ZeroDivisionError`. `foreign_call` carries the rest of the
// contract, including the argument marshalling, the ownership rule,
// and the two `NULL` checks that route a failure to the
// foreign failure edge (`foreign_fail.rs`).
pub(crate) fn emit_obj_method_call_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::ObjMethodCall {
        base, method, args, ..
    } = expr
    else {
        unreachable!()
    };
    foreign_call_emit::emit_method_call(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        base,
        method,
        args,
    )
}

// Part 3 of #1026 (PR 3b of #1082): `o[k]`. The evaluation order
// below is CPython's own -- base, then key -- and
// `foreign_call::emit_subscript` carries the rest of the contract,
// including why the packed key needs no `NULL` check of its own and
// why that leaves exactly one foreign failure edge.
pub(crate) fn emit_obj_subscript_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::ObjSubscript { base, index } = expr else {
        unreachable!()
    };
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let held_base = object_release::hold(context, module, rt, base, &base_scalar);
    let index_scalar = object_unbox::emit_pack_operand(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        index,
    );
    let held_index = object_release::hold(context, module, rt, index, &index_scalar);
    // A fresh native key (`o[str(i)]`) is copied by its packer, so it is
    // still this consumer's to release (`native_operand`).
    let native_index = crate::native_operand::hold(rt, index, index_scalar);
    let result =
        foreign_call::emit_subscript(context, builder, module, rt, base_scalar, index_scalar);
    held_index.release(builder, rt);
    native_index.release(context, builder, rt);
    held_base.release(builder, rt);
    result
}

// Part 1 of #1371: a comparison or identity test with a CPython
// object operand. Left, then right -- CPython's own order -- with
// the `Compare` arm's #638 protection of an `int` temporary across
// the right operand's evaluation; a `None` literal operand of an
// identity test is not evaluated at all (`foreign_compare` names
// CPython's `None` instead). `foreign_compare` carries the rest.
pub(crate) fn emit_obj_compare_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::ObjCompare { op, left, right } = expr else {
        unreachable!()
    };
    with_compare_operands(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        left,
        right,
        |l, r| foreign_compare::emit_compare(context, builder, module, rt, *op, l, r),
    )
}

/// The truth of an `ObjCompare` rich comparison in a condition, as an `i1`
/// (`if o == "x":`): the operands as [`emit_obj_compare_expr`] evaluates
/// them, then `foreign_compare::emit_compare_truth`, so no result object
/// is made and tested. `None` for any other expression, or an identity
/// test, which is already a native `bool`.
pub(crate) fn emit_obj_compare_condition<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Option<inkwell::values::IntValue<'ctx>> {
    let MirExpr::ObjCompare { op, left, right } = expr else {
        return None;
    };
    if matches!(op, pycc_mir::CmpOpKind::Is | pycc_mir::CmpOpKind::IsNot) {
        return None;
    }
    if let Some((text, other, text_first)) = text_equality(*op, left, right) {
        // The literal has no effect to order: only the object operand is
        // evaluated, and no `str` is made for the literal.
        let scalar = object_unbox::emit_pack_operand(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            other,
        );
        let held = object_release::hold(context, module, rt, other, &scalar);
        let truth = foreign_compare::emit_eq_text_truth(
            context, builder, module, rt, *op, scalar, text, text_first,
        );
        held.release(builder, rt);
        release_scalar_if_temporary(context, builder, rt, other, &scalar);
        return Some(truth);
    }
    Some(with_compare_operands(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        left,
        right,
        |l, r| foreign_compare::emit_compare_truth(context, builder, module, rt, *op, l, r),
    ))
}

/// Evaluates an `ObjCompare`'s operands, left then right, hands them to
/// `compare`, and releases them after it.
#[allow(clippy::too_many_arguments)]
fn with_compare_operands<'ctx, T>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    left: &MirExpr,
    right: &MirExpr,
    compare: impl FnOnce(Option<Scalar<'ctx>>, Option<Scalar<'ctx>>) -> T,
) -> T {
    let operand = |expr: &MirExpr| {
        (!matches!(expr, MirExpr::NoneLiteral)).then(|| {
            object_unbox::emit_pack_operand(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                expr,
            )
        })
    };
    let hold = |expr: &MirExpr, scalar: Option<Scalar<'ctx>>| {
        scalar.map(|scalar| object_release::hold(context, module, rt, expr, &scalar))
    };
    let l = operand(left);
    let held_l = hold(left, l);
    let pending_l = l.and_then(|l| push_pending_int_release_if_scalar_temporary(rt, left, &l));
    let r = operand(right);
    let held_r = hold(right, r);
    pop_pending_int_release(rt, pending_l);
    let result = compare(l, r);
    for held in [held_r, held_l].into_iter().flatten() {
        held.release(builder, rt);
    }
    for (expr, scalar) in [(left, l), (right, r)] {
        if let Some(scalar) = scalar {
            release_scalar_if_temporary(context, builder, rt, expr, &scalar);
        }
    }
    result
}

// Non-strict mode (#3): CPython's own operators on an object operand.
pub(crate) fn emit_obj_bin_op_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::ObjBinOp {
        op,
        in_place,
        left,
        right,
    } = expr
    else {
        unreachable!()
    };
    foreign_arith::emit_binop(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        *op,
        *in_place,
        left,
        right,
    )
}

// Part 2b of #1371: `item in container`. Item, then container --
// CPython's own order -- with the `ObjCompare` arm's #638
// protection of an `int` temporary item across the container's
// evaluation. `foreign_compare::emit_contains` carries the rest.
pub(crate) fn emit_obj_contains_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::ObjContains {
        negate,
        item,
        container,
    } = expr
    else {
        unreachable!()
    };
    let item_scalar =
        object_unbox::emit_pack_operand(context, builder, module, rt, user_functions, locals, item);
    let held_item = object_release::hold(context, module, rt, item, &item_scalar);
    if let Some(set_name) = boxed_text_set_name(item, container) {
        // `o in fs` for a frozenset of `str` read from a name: the native
        // set itself, borrowed (a name read has no effect to order and
        // nothing to release), with no CPython copy for an exact `str`.
        let set = emit_expr(context, builder, module, rt, user_functions, locals, set_name);
        let Scalar::Set(set) = set else {
            unreachable!("a frozenset name reads as a set")
        };
        let result = foreign_compare::emit_in_text_set(
            context,
            builder,
            module,
            rt,
            *negate,
            item_scalar,
            set,
        );
        held_item.release(builder, rt);
        release_scalar_if_temporary(context, builder, rt, item, &item_scalar);
        return result;
    }
    if crate::const_container::is_constant_sequence(container) {
        let container_ptr =
            crate::const_container::emit_cached_constant(context, builder, module, rt, user_functions, locals, container);
        let result = foreign_compare::emit_contains(
            context,
            builder,
            module,
            rt,
            *negate,
            item_scalar,
            Scalar::Object(container_ptr),
        );
        held_item.release(builder, rt);
        release_scalar_if_temporary(context, builder, rt, item, &item_scalar);
        return result;
    }
    let pending = push_pending_int_release_if_scalar_temporary(rt, item, &item_scalar);
    let container_scalar = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        container,
    );
    let held_container = object_release::hold(context, module, rt, container, &container_scalar);
    pop_pending_int_release(rt, pending);
    let result = foreign_compare::emit_contains(
        context,
        builder,
        module,
        rt,
        *negate,
        item_scalar,
        container_scalar,
    );
    held_container.release(builder, rt);
    held_item.release(builder, rt);
    release_scalar_if_temporary(context, builder, rt, item, &item_scalar);
    result
}

/// The name read inside `container` when an `ObjContains` tests an object
/// `item` against `ObjectBox(name)` of a `frozenset[str]` name.
fn boxed_text_set_name<'e>(item: &MirExpr, container: &'e MirExpr) -> Option<&'e MirExpr> {
    let MirExpr::ObjectBox(inner) = container else {
        return None;
    };
    let is_text_set = matches!(inner.ty(), pycc_mir::Ty::FrozenSet(element) if *element == pycc_mir::Ty::Str);
    (item.ty() == pycc_mir::Ty::Object && is_text_set && matches!(**inner, MirExpr::Name { .. }))
        .then_some(&**inner)
}

/// The string literal operand of an `==`/`!=` between it and an object
/// operand: `(text, the object operand, whether the literal is on the left)`.
fn text_equality<'e>(
    op: pycc_mir::CmpOpKind,
    left: &'e MirExpr,
    right: &'e MirExpr,
) -> Option<(&'e str, &'e MirExpr, bool)> {
    if !matches!(op, pycc_mir::CmpOpKind::Eq | pycc_mir::CmpOpKind::NotEq) {
        return None;
    }
    match (left, right) {
        (MirExpr::StringLiteral(text), other) if other.ty() == pycc_mir::Ty::Object => {
            Some((text, other, true))
        }
        (other, MirExpr::StringLiteral(text)) if other.ty() == pycc_mir::Ty::Object => {
            Some((text, other, false))
        }
        _ => None,
    }
}

// Part 2b of #1371: `o[a:b:c]`. `foreign_slice` owns the operand
// order, the `int`-temporary protection and the call.
pub(crate) fn emit_obj_slice_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::ObjSlice {
        base,
        start,
        stop,
        step,
    } = expr
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
        [start.as_deref(), stop.as_deref(), step.as_deref()],
        |base, bounds| foreign_slice::emit_slice(context, builder, module, rt, base, bounds),
    )
}

// Part 2d of #1371: a list display bound to an object slot. Each
// element in source order, every evaluated `int` temporary protected
// across the evaluations after it and released after the call, the
// `ObjSlice` arm's discipline. `foreign_call::emit_list` carries the
// rest.
// Non-strict mode (#3): a tuple, set or dict display with an object
// element, under the same discipline.
pub(crate) fn emit_obj_list_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let (MirExpr::ObjList { elements } | MirExpr::ObjDisplay { elements, .. }) = expr else {
        unreachable!()
    };
    let mut pendings = Vec::with_capacity(elements.len());
    let mut scalars = Vec::with_capacity(elements.len());
    let mut holds = Vec::with_capacity(elements.len());
    for element in elements {
        let scalar = object_unbox::emit_pack_operand(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            element,
        );
        holds.push(object_release::hold(context, module, rt, element, &scalar));
        pendings.push(push_pending_int_release_if_scalar_temporary(
            rt, element, &scalar,
        ));
        scalars.push(scalar);
    }
    for pending in pendings.into_iter().rev() {
        pop_pending_int_release(rt, pending);
    }
    let result = match expr {
        MirExpr::ObjDisplay { kind, .. } => {
            foreign_call::emit_display(context, builder, module, rt, *kind, &scalars)
        }
        _ => foreign_call::emit_list(context, builder, module, rt, &scalars),
    };
    for held in holds.into_iter().rev() {
        held.release(builder, rt);
    }
    for (element, scalar) in elements.iter().zip(&scalars) {
        release_scalar_if_temporary(context, builder, rt, element, scalar);
    }
    result
}

pub(crate) fn emit_obj_is_instance_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::ObjIsInstance { value, class } = expr else {
        unreachable!()
    };
    let value_scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let held_value = object_release::hold(context, module, rt, value, &value_scalar);
    let mut held_classes = Vec::new();
    // Every class expression first (a tuple's, in order), as CPython
    // evaluates the class argument before testing.
    let mut evaluate = |class: &pycc_mir::ObjIsInstanceClass| match class {
        pycc_mir::ObjIsInstanceClass::Object(class) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, class);
            held_classes.push(object_release::hold(context, module, rt, class, &scalar));
            foreign_compare::IsInstanceClass::Object(scalar)
        }
        pycc_mir::ObjIsInstanceClass::Builtin(builtin) => {
            foreign_compare::IsInstanceClass::Builtin(builtin.shim_code())
        }
        pycc_mir::ObjIsInstanceClass::Compiled(name) => {
            foreign_compare::IsInstanceClass::Compiled(name.clone())
        }
        pycc_mir::ObjIsInstanceClass::AnyOf(_) => {
            unreachable!("pycc_mir flattens a nested tuple of classes")
        }
    };
    let classes: Vec<_> = match class {
        pycc_mir::ObjIsInstanceClass::AnyOf(classes) => classes.iter().map(&mut evaluate).collect(),
        class => vec![evaluate(class)],
    };
    let result =
        foreign_compare::emit_isinstance_any(context, builder, module, rt, value_scalar, classes);
    for held in held_classes {
        held.release(builder, rt);
    }
    held_value.release(builder, rt);
    result
}

// Part 4 of #1026 (PR 4c of #1083): `x: tuple[float, float, float] =
// o` at module scope, and the same at any other fixed arity -- the
// PEP 585 variadic `tuple[float, ...]` stays refused and never
// reaches here. `foreign_len::emit_unpack_float_tuple` carries the
// contract -- why the out-slot is an array rather than a scalar,
// why the struct is reassembled here with `build_insert_value`, and
// why the arity travels as a call argument.
pub(crate) fn emit_obj_unpack_float_tuple_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::ObjUnpackFloatTuple { base, arity } = expr else {
        unreachable!()
    };
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let held = object_release::hold(context, module, rt, base, &base_scalar);
    let result =
        foreign_len::emit_unpack_float_tuple(context, builder, module, rt, base_scalar, *arity);
    held.release(builder, rt);
    result
}
