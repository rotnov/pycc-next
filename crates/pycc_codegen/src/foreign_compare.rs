//! Emission for `MirExpr::ObjCompare` and `MirExpr::ObjIsInstance` (Part 1
//! of #1371): comparisons, identity tests and `isinstance` with a CPython
//! object operand; and for `MirExpr::ObjContains` (Part 2b of #1371), a
//! membership test against a CPython object container.
//!
//! **Identity** (`is`/`is not`) is a pointer compare and emits no shim
//! call except to name CPython's `None` ([`EXT_OBJ_NONE_SYMBOL`], a
//! borrowed pointer to an immortal object). It cannot fail, so it has no
//! failure edge, and `exception::expression_can_set_exception` answers
//! `false` for it.
//!
//! **A rich comparison** is `PyObject_RichCompare` behind
//! [`EXT_OBJ_RICHCOMPARE_SYMBOL`]. A scalar operand is boxed by the same
//! `pycc_ext_obj_pack_*` helper a method-call argument uses
//! (`foreign_pack::packer_for`), and the shim helper consumes every packed
//! operand on every path -- the packer contract `foreign_call.rs` records --
//! so a failed packer needs no edge of its own and the operation has
//! exactly one: `foreign_fail.rs`'s, on a `NULL` result. The result is a
//! *new* reference: its consumer releases it when it is an unbound
//! temporary (`object_release.rs`, Part 1 of #1092), and a slot that binds
//! it owns it (`heap_rc`); a truth
//! context tests it through `foreign_len::emit_truthy` like any other object.
//!
//! **`isinstance`** is `PyObject_IsInstance` behind
//! [`EXT_OBJ_ISINSTANCE_SYMBOL`], whose `-1` takes the same failure edge.
//! A class compiled in this module (Part 7 of #1371) goes through
//! [`EXT_OBJ_ISINSTANCE_COMPILED_SYMBOL`] instead, with the class name as a
//! private global C string, and shares that edge.
//!
//! **Membership** (`in`/`not in`) is `PySequence_Contains` behind
//! [`EXT_OBJ_CONTAINS_SYMBOL`]. The item is always packed -- an object item
//! too, through `pycc_ext_obj_pack_object`'s new reference -- so the helper
//! consumes it on every path under the packer contract, and the container
//! is borrowed. Its `-1` takes the same single failure edge; `not in` is the
//! helper's `1`/`0` answer flipped.

use super::*;
use crate::foreign_attr::expect_object_pointer;
use crate::foreign_fail::{ForeignFailEdge, route_negative, route_null};
use crate::foreign_pack::{emit_pack, none_pointer, shim_fn};
use inkwell::builder::Builder;
use inkwell::values::PointerValue;
use pycc_mir::CmpOpKind;

/// CPython's rich-comparison selector (`Py_LT` .. `Py_GE`) for `op`, or
/// `None` for an identity test, which is no rich comparison at all.
fn rich_compare_selector(op: CmpOpKind) -> Option<u64> {
    match op {
        CmpOpKind::Lt => Some(0),
        CmpOpKind::LtE => Some(1),
        CmpOpKind::Eq => Some(2),
        CmpOpKind::NotEq => Some(3),
        CmpOpKind::Gt => Some(4),
        CmpOpKind::GtE => Some(5),
        CmpOpKind::Is | CmpOpKind::IsNot => None,
        // Part 2b of #1371: membership is its own node, `ObjContains`. It
        // must never join the `None` arm above, which means identity.
        CmpOpKind::In | CmpOpKind::NotIn => panic!(
            "pycc_codegen: internal error: a membership test reached `ObjCompare` -- \
             pycc_mir lowers it to `ObjContains`"
        ),
    }
}

/// The `PyObject *` for one operand, and whether a packer produced it (and
/// so the shim helper owns it). `None` is the unevaluated `None` literal of
/// an identity test.
fn operand_pointer<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    operand: Option<Scalar<'ctx>>,
) -> (PointerValue<'ctx>, bool) {
    match operand {
        None => (none_pointer(context, builder, module), false),
        Some(Scalar::Object(pointer)) => (pointer, false),
        Some(scalar) => (
            emit_pack(context, builder, module, scalar, "foreign_compare_operand"),
            true,
        ),
    }
}

/// Emits `left op right` once both operands are evaluated. `None` stands
/// for an unevaluated `None` literal (identity tests only).
pub(super) fn emit_compare<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    op: CmpOpKind,
    left: Option<Scalar<'ctx>>,
    right: Option<Scalar<'ctx>>,
) -> Scalar<'ctx> {
    let Some(selector) = rich_compare_selector(op) else {
        let (l, _) = operand_pointer(context, builder, module, left);
        let (r, _) = operand_pointer(context, builder, module, right);
        let predicate = if op == CmpOpKind::Is {
            inkwell::IntPredicate::EQ
        } else {
            inkwell::IntPredicate::NE
        };
        let same = builder
            .build_int_compare(predicate, l, r, "foreign_identity")
            .expect("build_int_compare should not fail on two pointers");
        let as_bool = builder
            .build_int_z_extend(same, context.i8_type(), "foreign_identity_bool")
            .expect("build_int_z_extend should not fail");
        return Scalar::Bool(as_bool);
    };
    let edge = ForeignFailEdge::for_current(builder);
    let (l, l_owned) = operand_pointer(context, builder, module, left);
    let (r, r_owned) = operand_pointer(context, builder, module, right);
    let owned = u64::from(l_owned) | (u64::from(r_owned) << 1);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i32_type = context.i32_type();
    let richcompare = shim_fn(
        module,
        EXT_OBJ_RICHCOMPARE_SYMBOL,
        ptr.fn_type(
            &[ptr.into(), ptr.into(), i32_type.into(), i32_type.into()],
            false,
        ),
    );
    let result = builder
        .build_call(
            richcompare,
            &[
                l.into(),
                r.into(),
                i32_type.const_int(selector, false).into(),
                i32_type.const_int(owned, false).into(),
            ],
            "foreign_compare",
        )
        .expect("build_call should not fail for pycc_ext_obj_richcompare")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_richcompare returns PyObject *")
        .into_pointer_value();
    route_null(
        context,
        builder,
        module,
        rt,
        edge,
        result,
        "foreign_compare",
    );
    Scalar::Object(result)
}

/// Emits the truth of `left op right` for a rich comparison (not an
/// identity test) once both operands are evaluated, as an `i1`: what
/// [`emit_compare`] followed by `foreign_len::emit_truthy` answers, in one
/// shim call and without a result object for two `str` operands. A `-1`
/// takes the failure edge.
pub(super) fn emit_compare_truth<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    op: CmpOpKind,
    left: Option<Scalar<'ctx>>,
    right: Option<Scalar<'ctx>>,
) -> inkwell::values::IntValue<'ctx> {
    let selector = rich_compare_selector(op)
        .expect("pycc_codegen: internal error: an identity test has no rich comparison");
    let edge = ForeignFailEdge::for_current(builder);
    let (l, l_owned) = operand_pointer(context, builder, module, left);
    let (r, r_owned) = operand_pointer(context, builder, module, right);
    let owned = u64::from(l_owned) | (u64::from(r_owned) << 1);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i32_type = context.i32_type();
    let compare = shim_fn(
        module,
        crate::ext::EXT_OBJ_COMPARE_BOOL_SYMBOL,
        i32_type.fn_type(
            &[ptr.into(), ptr.into(), i32_type.into(), i32_type.into()],
            false,
        ),
    );
    let status = builder
        .build_call(
            compare,
            &[
                l.into(),
                r.into(),
                i32_type.const_int(selector, false).into(),
                i32_type.const_int(owned, false).into(),
            ],
            "foreign_compare_truth",
        )
        .expect("build_call should not fail for pycc_ext_obj_compare_bool")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_compare_bool returns int")
        .into_int_value();
    route_negative(
        context,
        builder,
        module,
        rt,
        edge,
        status,
        "foreign_compare_truth",
    );
    builder
        .build_int_truncate(status, context.bool_type(), "foreign_compare_bit")
        .expect("build_int_truncate should not fail")
}

/// The truth of `object == text` / `object != text` (`op`) for an evaluated
/// object operand (borrowed) and a string literal, `text_first` when the
/// literal is the left operand: [`crate::ext::EXT_OBJ_EQ_TEXT_SYMBOL`] with
/// the literal's bytes as a private constant, so no `str` is made for it.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_eq_text_truth<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    op: CmpOpKind,
    object: Scalar<'ctx>,
    text: &str,
    text_first: bool,
) -> inkwell::values::IntValue<'ctx> {
    let selector = rich_compare_selector(op)
        .expect("pycc_codegen: internal error: an identity test has no rich comparison");
    let edge = ForeignFailEdge::for_current(builder);
    let object = expect_object_pointer(object);
    let bytes = text.as_bytes();
    let global = module.add_global(
        context.i8_type().array_type(bytes.len() as u32),
        None,
        "eq_text_lit",
    );
    global.set_initializer(&context.const_string(bytes, false));
    global.set_constant(true);
    global.set_linkage(inkwell::module::Linkage::Private);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i32_type = context.i32_type();
    let i64_type = context.i64_type();
    let eq_text = shim_fn(
        module,
        crate::ext::EXT_OBJ_EQ_TEXT_SYMBOL,
        i32_type.fn_type(
            &[
                ptr.into(),
                ptr.into(),
                i64_type.into(),
                i32_type.into(),
                i32_type.into(),
            ],
            false,
        ),
    );
    let status = builder
        .build_call(
            eq_text,
            &[
                object.into(),
                global.as_pointer_value().into(),
                i64_type.const_int(bytes.len() as u64, false).into(),
                i32_type.const_int(selector, false).into(),
                i32_type.const_int(u64::from(text_first), false).into(),
            ],
            "foreign_eq_text",
        )
        .expect("build_call should not fail for pycc_ext_obj_eq_text")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_eq_text returns int")
        .into_int_value();
    route_negative(context, builder, module, rt, edge, status, "foreign_eq_text");
    builder
        .build_int_truncate(status, context.bool_type(), "foreign_eq_text_bit")
        .expect("build_int_truncate should not fail")
}

/// `item in set` (`item not in set` when `negate`) for an evaluated object
/// item and a native frozenset of `str`, both borrowed:
/// [`crate::ext::EXT_OBJ_IN_TEXT_SET_SYMBOL`], which looks an exact `str`
/// up by its bytes with no CPython copy of the set.
pub(super) fn emit_in_text_set<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    negate: bool,
    item: Scalar<'ctx>,
    set: PointerValue<'ctx>,
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i32_type = context.i32_type();
    let item = expect_object_pointer(item);
    let in_set = shim_fn(
        module,
        crate::ext::EXT_OBJ_IN_TEXT_SET_SYMBOL,
        i32_type.fn_type(&[ptr.into(), ptr.into()], false),
    );
    let status = builder
        .build_call(in_set, &[item.into(), set.into()], "foreign_in_text_set")
        .expect("build_call should not fail for pycc_ext_obj_in_text_set")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_in_text_set returns int")
        .into_int_value();
    route_negative(
        context,
        builder,
        module,
        rt,
        edge,
        status,
        "foreign_in_text_set",
    );
    let found = builder
        .build_int_truncate(status, context.i8_type(), "foreign_in_text_set_bool")
        .expect("build_int_truncate should not fail");
    let answer = builder
        .build_xor(
            found,
            context.i8_type().const_int(u64::from(negate), false),
            "foreign_in_text_set_answer",
        )
        .expect("build_xor should not fail on an i8");
    Scalar::Bool(answer)
}

/// Emits `item in container` (or `item not in container` when `negate`)
/// once both operands are evaluated.
pub(super) fn emit_contains<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    negate: bool,
    item: Scalar<'ctx>,
    container: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i32_type = context.i32_type();
    let container_ptr = expect_object_pointer(container);
    let item_ptr = emit_pack(context, builder, module, item, "foreign_contains_item");
    let contains = shim_fn(
        module,
        EXT_OBJ_CONTAINS_SYMBOL,
        i32_type.fn_type(&[ptr.into(), ptr.into()], false),
    );
    let status = builder
        .build_call(
            contains,
            &[container_ptr.into(), item_ptr.into()],
            "foreign_contains",
        )
        .expect("build_call should not fail for pycc_ext_obj_contains")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_contains returns int")
        .into_int_value();
    route_negative(
        context,
        builder,
        module,
        rt,
        edge,
        status,
        "foreign_contains",
    );
    let found = builder
        .build_int_truncate(status, context.i8_type(), "foreign_contains_bool")
        .expect("build_int_truncate should not fail");
    let answer = builder
        .build_xor(
            found,
            context.i8_type().const_int(u64::from(negate), false),
            "foreign_contains_answer",
        )
        .expect("build_xor should not fail on an i8");
    Scalar::Bool(answer)
}

/// The evaluated class operand of an `isinstance` test.
pub(super) enum IsInstanceClass<'ctx> {
    /// A builtin class, by its `pycc_ext_obj_isinstance` selector.
    Builtin(u64),
    /// An evaluated object-typed class expression.
    Object(Scalar<'ctx>),
    /// A class compiled in this module, by name (Part 7 of #1371).
    Compiled(String),
}

/// Emits `isinstance(value, class)` once both operands are evaluated.
pub(super) fn emit_isinstance<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    value: Scalar<'ctx>,
    class: IsInstanceClass<'ctx>,
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i32_type = context.i32_type();
    let value_ptr = expect_object_pointer(value);
    let (class_ptr, builtin) = match class {
        IsInstanceClass::Builtin(selector) => (ptr.const_null(), selector),
        IsInstanceClass::Object(class) => (expect_object_pointer(class), 0),
        IsInstanceClass::Compiled(name) => {
            // Another module's class: its own entry tests its own type.
            let (symbol, name) =
                crate::sibling_class::route(EXT_OBJ_ISINSTANCE_COMPILED_SYMBOL, &name);
            let name_ptr = builder
                .build_global_string_ptr(&name, &format!("pycc_isinstance_class_{name}"))
                .expect("build_global_string_ptr should not fail")
                .as_pointer_value();
            let compiled = shim_fn(
                module,
                &symbol,
                i32_type.fn_type(&[ptr.into(), ptr.into()], false),
            );
            let status = builder
                .build_call(
                    compiled,
                    &[value_ptr.into(), name_ptr.into()],
                    "foreign_isinstance",
                )
                .expect("build_call should not fail for pycc_ext_obj_isinstance_compiled")
                .try_as_basic_value()
                .expect_basic("pycc_ext_obj_isinstance_compiled returns int")
                .into_int_value();
            return finish_isinstance(context, builder, module, rt, edge, status);
        }
    };
    let isinstance = shim_fn(
        module,
        EXT_OBJ_ISINSTANCE_SYMBOL,
        i32_type.fn_type(&[ptr.into(), ptr.into(), i32_type.into()], false),
    );
    let status = builder
        .build_call(
            isinstance,
            &[
                value_ptr.into(),
                class_ptr.into(),
                i32_type.const_int(builtin, false).into(),
            ],
            "foreign_isinstance",
        )
        .expect("build_call should not fail for pycc_ext_obj_isinstance")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_isinstance returns int")
        .into_int_value();
    finish_isinstance(context, builder, module, rt, edge, status)
}

/// Emits `isinstance(value, (c1, c2, ...))` once every operand is
/// evaluated: each class tested in order, the first `True` answering
/// (CPython's `PyObject_IsInstance` over a tuple). One class is
/// [`emit_isinstance`] itself.
pub(super) fn emit_isinstance_any<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    value: Scalar<'ctx>,
    classes: Vec<IsInstanceClass<'ctx>>,
) -> Scalar<'ctx> {
    let count = classes.len();
    if count == 1 {
        let class = classes.into_iter().next().expect("one class");
        return emit_isinstance(context, builder, module, rt, value, class);
    }
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("an isinstance is emitted inside a function");
    let done = context.append_basic_block(function, "isinstance_any_done");
    let mut incoming = Vec::with_capacity(count);
    for (index, class) in classes.into_iter().enumerate() {
        let Scalar::Bool(answer) = emit_isinstance(context, builder, module, rt, value, class)
        else {
            unreachable!("emit_isinstance answers a bool")
        };
        let at = builder.get_insert_block().expect("a block");
        incoming.push((answer, at));
        if index + 1 == count {
            builder
                .build_unconditional_branch(done)
                .expect("build_unconditional_branch should not fail");
        } else {
            let next = context.append_basic_block(function, "isinstance_any_next");
            let hit = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    answer,
                    context.i8_type().const_zero(),
                    "isinstance_any_hit",
                )
                .expect("build_int_compare should not fail");
            builder
                .build_conditional_branch(hit, done, next)
                .expect("build_conditional_branch should not fail");
            builder.position_at_end(next);
        }
    }
    builder.position_at_end(done);
    let phi = builder
        .build_phi(context.i8_type(), "isinstance_any")
        .expect("build_phi should not fail");
    for (answer, at) in &incoming {
        phi.add_incoming(&[(answer, *at)]);
    }
    Scalar::Bool(phi.as_basic_value().into_int_value())
}

/// Routes an `isinstance` helper's `-1` to the operation's failure edge and
/// narrows its `1`/`0` to the `bool` result.
fn finish_isinstance<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    edge: ForeignFailEdge<'ctx>,
    status: inkwell::values::IntValue<'ctx>,
) -> Scalar<'ctx> {
    route_negative(
        context,
        builder,
        module,
        rt,
        edge,
        status,
        "foreign_isinstance",
    );
    let as_bool = builder
        .build_int_truncate(status, context.i8_type(), "foreign_isinstance_bool")
        .expect("build_int_truncate should not fail");
    Scalar::Bool(as_bool)
}
