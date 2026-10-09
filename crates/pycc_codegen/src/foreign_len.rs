//! Emission for `len(o)`, truth testing, the `float(o)`, `int(o)` and
//! `str(o)` conversions, the f-string rendering `format(o, '')` and the
//! fixed-arity all-`float` tuple unpack on a CPython object value (Part 3 of
//! #1026, PR 3a of #1082; Part 4 of #1026, PRs 4a, 4b and 4c of #1083;
//! #1340).
//!
//! The third sibling of `foreign_attr.rs` and `foreign_call.rs`, carved out
//! of `lib.rs` for the same reason (AGENTS.md's "Keep source files
//! decomposable"; the tracker for the rest of `lib.rs` is #545). The two
//! operations share a module because they share everything that matters at
//! this layer: each is one call to a fixed shim helper that answers a
//! *scalar* rather than a `PyObject *`, and each reports failure as `-1`
//! rather than as `NULL`, so each routes its failure through
//! `foreign_fail::route_negative` rather than `route_null`. PR 4a's
//! `float(o)` is the third member of that family and lives here for the
//! same reason; `bool(o)` needs no emitter of its own at
//! all, because it is exactly [`emit_truthy`] widened to a `Scalar::Bool`.
//! PR 4b's `int(o)` and `str(o)` are the fourth and fifth: identical shape
//! again, differing only in the out-slot's type -- an `i64` holding a D-141
//! encoded word, exactly like [`emit_len`]'s, and a pointer holding the
//! `PyStrObj *` a `str` literal's own emission produces.
//!
//! PR 4c's [`emit_unpack_float_tuple`] is the sixth, and the first whose
//! out-slot is not a single scalar: the helper fills an array of `arity`
//! `double`s, which this module then rebuilds into the D-115/D-116 by-value
//! LLVM struct the fixed-arity all-`float` tuple annotation denotes (the
//! PEP 585 variadic `tuple[float, ...]` stays refused, so no arity is ever
//! unknown here). The family resemblance is otherwise exact -- one call to
//! a fixed shim helper, `-1` for failure, no reference escaping -- which is
//! why it lives here and not in a module of its own.
//!
//! **Ownership** (`docs/RUNTIME.md`). None of these helpers lets a reference
//! escape -- `pycc_ext_obj_len` answers a D-141 encoded `int` word,
//! `pycc_ext_obj_truthy` answers a C `int`, `pycc_ext_obj_to_float`
//! answers a `double`, `pycc_ext_obj_to_int` answers a D-141 encoded word
//! and `pycc_ext_obj_to_str` answers a `PyStrObj *` copied out of CPython's
//! own buffer (as does #1340's `pycc_ext_obj_format`, for `format(o, '')`),
//! and `pycc_ext_obj_unpack_float_tuple` writes plain `double`s
//! through an out-param -- each after releasing the CPython temporary its
//! conversion protocol handed it, on *every* exit rather than only the
//! successful one -- and none touches its operand's refcount. So unlike an
//! attribute load or a method call, none of these adds anything to the #1092
//! leak-only set: there is nothing to leak in compiled code and nothing left
//! to release.
//!
//! **Failure** (`docs/RUNTIME.md`). Every helper here leaves *CPython's*
//! error indicator set, which pycc's own pending-exception guard (D-173) cannot
//! see, so each arm below emits its own check through
//! `foreign_fail::route_negative`: inside `pycc_ext_module_exec` with no
//! module-level `try` enclosing it, it returns [`EXT_MODULE_EXEC_FAILED`];
//! anywhere else it bridges the exception into pycc's pending state and
//! branches to the innermost exception target (#1316, Part 1 of #1096), so
//! a module-level `try` catches it.

use super::*;
use crate::foreign_attr::{expect_module_exec_entry, expect_object_pointer};
use crate::foreign_fail::{ForeignFailEdge, route_negative};
use inkwell::builder::Builder;
use inkwell::values::{IntValue, PointerValue};

/// Declares the shim's `int pycc_ext_obj_len(PyObject *, long long *)` once
/// per module, returning the existing declaration on every later call --
/// `foreign_attr.rs`'s `obj_getattr_fn` pattern exactly, and for the same
/// reason: a second declaration of one name is an LLVM module-verifier
/// error.
fn obj_len_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_OBJ_LEN_SYMBOL) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    module.add_function(
        EXT_OBJ_LEN_SYMBOL,
        context.i32_type().fn_type(&[ptr.into(), ptr.into()], false),
        None,
    )
}

/// Declares the shim's `int pycc_ext_obj_truthy(PyObject *)` once per
/// module, on [`obj_len_fn`]'s pattern and for its reason.
fn obj_truthy_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_OBJ_TRUTHY_SYMBOL) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    module.add_function(
        EXT_OBJ_TRUTHY_SYMBOL,
        context.i32_type().fn_type(&[ptr.into()], false),
        None,
    )
}

/// Declares the shim's `int pycc_ext_obj_to_float(PyObject *, double *)`
/// once per module, on [`obj_len_fn`]'s pattern and for its reason.
fn obj_to_float_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_OBJ_TO_FLOAT_SYMBOL) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    module.add_function(
        EXT_OBJ_TO_FLOAT_SYMBOL,
        context.i32_type().fn_type(&[ptr.into(), ptr.into()], false),
        None,
    )
}

/// Declares the shim's `int pycc_ext_obj_to_int(PyObject *, long long *)`
/// once per module, on [`obj_len_fn`]'s pattern and for its reason.
fn obj_to_int_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_OBJ_TO_INT_SYMBOL) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    module.add_function(
        EXT_OBJ_TO_INT_SYMBOL,
        context.i32_type().fn_type(&[ptr.into(), ptr.into()], false),
        None,
    )
}

/// Declares the shim's `int pycc_ext_obj_to_str(PyObject *, void **)` once
/// per module, on [`obj_len_fn`]'s pattern and for its reason.
fn obj_to_str_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_OBJ_TO_STR_SYMBOL) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    module.add_function(
        EXT_OBJ_TO_STR_SYMBOL,
        context.i32_type().fn_type(&[ptr.into(), ptr.into()], false),
        None,
    )
}

/// Declares the shim's `int pycc_ext_obj_format(PyObject *, void **)` once
/// per module (#1340), on [`obj_len_fn`]'s pattern and for its reason.
fn obj_format_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_OBJ_FORMAT_SYMBOL) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    module.add_function(
        EXT_OBJ_FORMAT_SYMBOL,
        context.i32_type().fn_type(&[ptr.into(), ptr.into()], false),
        None,
    )
}

/// Declares the shim's
/// `int pycc_ext_obj_unpack_float_tuple(PyObject *, long long, double *)`
/// once per module, on [`obj_len_fn`]'s pattern and for its reason.
fn obj_unpack_float_tuple_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_OBJ_UNPACK_FLOAT_TUPLE_SYMBOL) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    module.add_function(
        EXT_OBJ_UNPACK_FLOAT_TUPLE_SYMBOL,
        context
            .i32_type()
            .fn_type(&[ptr.into(), context.i64_type().into(), ptr.into()], false),
        None,
    )
}

/// Allocates one `slot_ty` out-slot named `name` in the *entry block* of
/// `entry_fn`, leaving the builder positioned exactly where it was.
///
/// The single-slot twin of `foreign_call.rs`'s `alloca_in_entry_block`, and
/// it exists for that function's documented reason rather than for tidiness:
/// an `alloca` is reclaimed only when its function returns, so emitting one
/// at the call site would make `while c: n = len(gc)` at module scope grow
/// the hosting interpreter's stack without bound.
fn out_slot_in_entry_block<'ctx>(
    builder: &Builder<'ctx>,
    entry_fn: FunctionValue<'ctx>,
    slot_ty: impl inkwell::types::BasicType<'ctx>,
    name: &str,
) -> PointerValue<'ctx> {
    super::build_at_entry_block(builder, entry_fn, |b| {
        b.build_alloca(slot_ty, name)
            .expect("build_alloca should not fail")
    })
}

/// Emits one `len(o)` against a CPython object and yields the length as an
/// already-D-141-encoded [`Scalar::Int`].
///
/// The encode happens inside the shim rather than here, which is the whole
/// reason this is not a `Ty::Object` arm bolted onto `emit_expr`'s scalar
/// `len` dispatch: that path produces a *raw* `i64` and re-tags it at the
/// call site, while `pycc_ext_obj_len` hands back a finished word through an
/// out-parameter. Fusing `PyObject_Size` and `pycc_rt_ext_int_encode` behind
/// one `-1` return also means one failure edge is emitted here instead of
/// two -- see the C side's own comment for why the encode arm is unreachable
/// for a real container.
///
/// # Failure edge
///
/// `foreign_fail::route_negative`'s: the module-exec return in a module
/// body outside every module-level `try`, the bridge plus an immediate
/// branch to the innermost exception target anywhere else (#1316, #1096). The out-slot is
/// hoisted into the entry block of whichever function that is.
pub(super) fn emit_len<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let entry_fn = edge.function();
    let len_fn = obj_len_fn(context, module);
    let base_ptr = expect_object_pointer(base);
    let out = out_slot_in_entry_block(builder, entry_fn, context.i64_type(), "foreign_len_out");
    let status = builder
        .build_call(len_fn, &[base_ptr.into(), out.into()], "foreign_len")
        .expect("build_call should not fail for pycc_ext_obj_len")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_len returns int")
        .into_int_value();
    route_negative(context, builder, module, rt, edge, status, "foreign_len");
    let encoded = builder
        .build_load(context.i64_type(), out, "foreign_len_value")
        .expect("build_load should not fail")
        .into_int_value();
    Scalar::Int(encoded)
}

/// Emits one truth test against a CPython object and yields the `i1` an
/// `if`/`while`/comprehension-guard branch consumes.
///
/// Called from `lib.rs`'s `truthy`, whose `Scalar::Object` arm used to
/// panic: `pycc_types` refused a `Ty::Object` condition outright, precisely
/// *because* codegen had no answer here. PR 3a deleted those ten refusals
/// (`crates/pycc_types/src/foreign.rs`'s module documentation records it),
/// so this is the answer they were waiting on.
///
/// `PyObject_IsTrue` runs the operand's own `__bool__` or `__len__`, so it
/// can raise; the `-1` status takes [`emit_len`]'s failure edge. The
/// success values are `0` and `1`, which the truncation to `i1` below maps
/// exactly. The edge's branch is immediate, which matters most here: the
/// condition-position caller has no expression guard after it.
///
/// `MirExpr::Not` reaches here too since Part 10 of #1371: `not o` is this
/// truth test inverted into a native `bool`, its `-1` taking the same
/// immediate failure edge.
pub(super) fn emit_truthy<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    object: PointerValue<'ctx>,
) -> IntValue<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let truthy_fn = obj_truthy_fn(context, module);
    let status = builder
        .build_call(truthy_fn, &[object.into()], "foreign_truthy")
        .expect("build_call should not fail for pycc_ext_obj_truthy")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_truthy returns int")
        .into_int_value();
    route_negative(context, builder, module, rt, edge, status, "foreign_truthy");
    builder
        .build_int_truncate(status, context.bool_type(), "foreign_truthy_bit")
        .expect("build_int_truncate should not fail")
}

/// Emits one `float(o)` against a CPython object and yields the converted
/// value as a [`Scalar::Float`].
///
/// `lib.rs`'s `to_float` panics on a `Scalar::Object` -- it knows only how
/// to widen pycc's own scalars -- so the `float` builtin's emission branches
/// on the operand before reaching it and lands here instead.
///
/// **This is an explicit conversion, not an implicit boundary crossing.**
/// D-244 rule 7 closes the type boundary at the *thunk export seam*, where
/// a value crosses implicitly and the annotation is the whole contract.
/// `float(o)` names its destination type in user source, so running
/// CPython's own `PyNumber_Float` protocol behind [`EXT_OBJ_TO_FLOAT_SYMBOL`]
/// is exactly what the author asked for. The same paragraph is recorded on
/// that constant, in the helper's C comment, and in `docs/TYPE_SYSTEM.md`'s
/// `object` row.
///
/// The shim releases the reference `PyNumber_Float` produces on every exit,
/// so nothing here adds to the #1092 leak-only set. Failure is `-1` with
/// CPython's error indicator set, taking [`emit_len`]'s failure edge --
/// `PyNumber_Float` raises `TypeError` for an operand with
/// no conversion and `ValueError` for an unparseable string, neither of
/// which pycc can rule out at compile time for an opaque pointee.
pub(super) fn emit_to_float<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let entry_fn = edge.function();
    let to_float_fn = obj_to_float_fn(context, module);
    let base_ptr = expect_object_pointer(base);
    let out = out_slot_in_entry_block(
        builder,
        entry_fn,
        context.f64_type(),
        "foreign_to_float_out",
    );
    let status = builder
        .build_call(
            to_float_fn,
            &[base_ptr.into(), out.into()],
            "foreign_to_float",
        )
        .expect("build_call should not fail for pycc_ext_obj_to_float")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_to_float returns int")
        .into_int_value();
    route_negative(
        context,
        builder,
        module,
        rt,
        edge,
        status,
        "foreign_to_float",
    );
    let value = builder
        .build_load(context.f64_type(), out, "foreign_to_float_value")
        .expect("build_load should not fail")
        .into_float_value();
    Scalar::Float(value)
}

/// Emits one `int(o)` against a CPython object and yields the result as an
/// already-D-141-encoded [`Scalar::Int`].
///
/// [`emit_len`]'s shape exactly, and for its reason: `pycc_ext_obj_to_int`
/// fuses `PyNumber_Long`, CPython's own overflow check and
/// `pycc_rt_ext_int_encode` behind one `-1` return, so a finished word
/// arrives through the out-parameter and one failure edge is emitted here
/// instead of three. Nothing is re-tagged at this call site -- unlike
/// `lib.rs`'s scalar `len`, which receives a *raw* `i64`.
///
/// **This is an explicit conversion, not an implicit boundary crossing** --
/// [`emit_to_float`]'s paragraph, unchanged, recorded on
/// [`EXT_OBJ_TO_INT_SYMBOL`], in the helper's C comment and in
/// `docs/TYPE_SYSTEM.md`'s `object` row. In particular this is *not*
/// `pycc_ext_unpack_int_at`: that helper's `PyBool_Check`/`PyLong_Check`
/// guards exist because the thunk seam is closed, and an explicit
/// conversion is the seam where CPython's own answers are the contract.
///
/// Failure is `-1` with CPython's error indicator set, taking
/// [`emit_len`]'s failure edge: `PyNumber_Long` raises
/// for an operand with no integer conversion and for an unparseable string,
/// and a result outside pycc's inline-integer range raises `OverflowError`
/// (#1040) -- none of which pycc can rule out at compile time for an opaque
/// pointee.
pub(super) fn emit_to_int<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let entry_fn = edge.function();
    let to_int_fn = obj_to_int_fn(context, module);
    let base_ptr = expect_object_pointer(base);
    let out = out_slot_in_entry_block(builder, entry_fn, context.i64_type(), "foreign_to_int_out");
    let status = builder
        .build_call(to_int_fn, &[base_ptr.into(), out.into()], "foreign_to_int")
        .expect("build_call should not fail for pycc_ext_obj_to_int")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_to_int returns int")
        .into_int_value();
    route_negative(context, builder, module, rt, edge, status, "foreign_to_int");
    let encoded = builder
        .build_load(context.i64_type(), out, "foreign_to_int_value")
        .expect("build_load should not fail")
        .into_int_value();
    Scalar::Int(encoded)
}

/// Emits one `str(o)` against a CPython object and yields the result as a
/// [`Scalar::Str`].
///
/// [`emit_to_int`]'s shape with a pointer out-slot. The handle the shim
/// writes comes from `pycc_rt_str_from_literal` -- the identical call
/// `emit_string_literal` makes for a `str` literal -- so it arrives as this
/// module's own reference at refcount 1 and every downstream consumer
/// (`to_str`'s `Scalar::Str(v) => v`, an f-string, a `print`) handles it
/// exactly as it handles a literal-derived one. No new ownership rule is
/// needed in this crate; the load-bearing ordering (copy before release)
/// lives on the C side, where the buffer belongs to the `PyObject_Str`
/// result.
///
/// **This is an explicit conversion, not an implicit boundary crossing** --
/// [`emit_to_float`]'s paragraph, unchanged. `PyObject_Str` *is* `str()`.
///
/// Failure is `-1` with CPython's error indicator set, taking
/// [`emit_len`]'s failure edge: a `__str__` that raises
/// and a result holding a lone surrogate (no UTF-8 encoding) are both real
/// and neither is decidable at compile time for an opaque pointee.
pub(super) fn emit_to_str<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
) -> Scalar<'ctx> {
    Scalar::Str(emit_to_str_pointer(context, builder, module, rt, base))
}

/// [`emit_to_str`] yielding the bare `PyStrObj *` rather than a
/// [`Scalar::Str`], for a caller that writes the text straight out: the
/// write phase of `print(o)` (#1340, `string_render.rs`).
pub(super) fn emit_to_str_pointer<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
) -> PointerValue<'ctx> {
    let helper = obj_to_str_fn(context, module);
    emit_text_conversion(context, builder, module, rt, base, helper, "foreign_to_str")
}

/// Emits one f-string interpolation `f"{o}"` of a CPython object and yields
/// the text as a bare `PyStrObj *` (#1340).
///
/// [`emit_to_str`] exactly, reaching `pycc_ext_obj_format` instead: CPython
/// renders an interpolation as `format(o, '')`, which calls the operand's
/// `__format__` rather than its `__str__`, so the two differ whenever a
/// class overrides `__format__`. The ownership (a refcount-1 handle from
/// `pycc_rt_str_from_literal`) and the failure edge (a raising `__format__`
/// answers `-1`) are [`emit_to_str`]'s.
pub(super) fn emit_format<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
) -> PointerValue<'ctx> {
    let helper = obj_format_fn(context, module);
    emit_text_conversion(context, builder, module, rt, base, helper, "foreign_format")
}

/// The shared body of [`emit_to_str`] and [`emit_format`]: one call to a
/// shim helper of shape `int (PyObject *, void **)` that writes a pycc
/// `PyStrObj *` through a pointer out-slot, routed through the failure
/// edge, yielding the loaded pointer. `label` prefixes every emitted value
/// and block name, so the two helpers stay distinguishable in the IR.
fn emit_text_conversion<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
    helper: FunctionValue<'ctx>,
    label: &str,
) -> PointerValue<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let entry_fn = edge.function();
    let base_ptr = expect_object_pointer(base);
    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    let out = out_slot_in_entry_block(builder, entry_fn, ptr_ty, &format!("{label}_out"));
    let status = builder
        .build_call(helper, &[base_ptr.into(), out.into()], label)
        .expect("build_call should not fail for a text-conversion shim helper")
        .try_as_basic_value()
        .expect_basic("a text-conversion shim helper returns int")
        .into_int_value();
    route_negative(context, builder, module, rt, edge, status, label);
    builder
        .build_load(ptr_ty, out, &format!("{label}_value"))
        .expect("build_load should not fail")
        .into_pointer_value()
}

/// Emits the unpack of a CPython object into a fixed-arity all-`float`
/// tuple and yields the assembled [`Scalar::Tuple`] (Part 4 of #1026, PR 4c
/// of #1083).
///
/// The one emitter here whose result is an *aggregate*. D-115 holds a tuple
/// as a by-value LLVM struct of fixed-width scalars, so the shim cannot
/// hand one back through a scalar out-parameter: the out-slot is an
/// `[arity x double]` array, and the struct is reassembled here field by
/// field with `build_insert_value` -- exactly how `ext_thunk.rs` rebuilds a
/// tuple parameter out of its flattened slots, and for the same reason
/// (pycc's aggregate convention is not the platform C struct ABI, so no
/// aggregate may cross this seam).
///
/// `arity` is passed to the helper as an `i64` argument rather than baked
/// into the symbol: the admission rule is any fixed arity with every
/// element `float`, so nothing on either side may hard-code a three.
///
/// **Strict container, converting elements** -- the paragraph
/// [`EXT_OBJ_UNPACK_FLOAT_TUPLE_SYMBOL`] records. The runtime object's
/// items are never type-checked by pycc; a bad item fails at run time with
/// whatever exception CPython's own `PyNumber_Float` raises, on
/// the module body's foreign failure edge together with a wrong
/// container type and a wrong arity.
///
/// This emitter alone keeps the module-exec entry assertion: `pycc_types`
/// admits this shape only at a *module-level* annotated assignment.
pub(super) fn emit_unpack_float_tuple<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
    arity: usize,
) -> Scalar<'ctx> {
    let entry_fn = expect_module_exec_entry(builder);
    let edge = ForeignFailEdge::ModuleExec(entry_fn);
    let unpack_fn = obj_unpack_float_tuple_fn(context, module);
    let base_ptr = expect_object_pointer(base);
    let element_ty = context.f64_type();
    let array_ty = element_ty.array_type(u32::try_from(arity).expect("a tuple's arity fits a u32"));
    let out = out_slot_in_entry_block(
        builder,
        entry_fn,
        array_ty,
        "foreign_unpack_float_tuple_out",
    );
    let status = builder
        .build_call(
            unpack_fn,
            &[
                base_ptr.into(),
                context.i64_type().const_int(arity as u64, false).into(),
                out.into(),
            ],
            "foreign_unpack_float_tuple",
        )
        .expect("build_call should not fail for pycc_ext_obj_unpack_float_tuple")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_unpack_float_tuple returns int")
        .into_int_value();
    route_negative(
        context,
        builder,
        module,
        rt,
        edge,
        status,
        "foreign_unpack_float_tuple",
    );
    let struct_ty = ty_to_basic_type(
        context,
        pycc_mir::Ty::Tuple(Box::new(vec![pycc_mir::Ty::Float; arity])),
    )
    .into_struct_type();
    let mut aggregate = struct_ty.get_undef();
    for field in 0..arity {
        // The out-slot is one `[arity x double]` allocation, so each field
        // is a GEP into it rather than its own slot: one `alloca` hoisted
        // into the entry block keeps a module-scope loop around this
        // assignment from growing the host's stack, which is
        // `out_slot_in_entry_block`'s whole reason for existing.
        let element_ptr = unsafe {
            builder
                .build_in_bounds_gep(
                    array_ty,
                    out,
                    &[
                        context.i32_type().const_zero(),
                        context.i32_type().const_int(field as u64, false),
                    ],
                    "foreign_unpack_float_tuple_elem",
                )
                .expect("build_in_bounds_gep should not fail for a constant array index")
        };
        let element = builder
            .build_load(element_ty, element_ptr, "foreign_unpack_float_tuple_value")
            .expect("build_load should not fail");
        aggregate = builder
            .build_insert_value(aggregate, element, field as u32, "tuple_insert")
            .expect("build_insert_value should not fail for a well-typed tuple field")
            .into_struct_value();
    }
    Scalar::Tuple(aggregate)
}

#[cfg(test)]
#[path = "foreign_len_tests.rs"]
mod tests;
