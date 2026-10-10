//! memoryview/buffer element reads, lengths and allocation.

use super::*;

// Part 2 of #1027: `b[i]` on a `pycc build --ext` export's
// `memoryview` parameter. Base then index, CPython's own order and
// the list-subscript arm's own, and then a single call: the bounds
// check, the D-173 `IndexError` raise and the element read all live
// inside `pycc_rt_buffer_f64_get`. No GEP and no comparison is
// emitted here, deliberately -- an inline check would be IR this
// crate's own tests could only observe as text, where the runtime
// helper is directly unit-testable in `pycc_rt`.
//
// The index is decoded exactly as a list index is: `Ty::Int` (or
// `bool`, per D-086) arrives D-141 encoded, and `build_untag_checked`
// is what rejects a bigint or malformed word. The raise this node
// can leave pending is declared by `expression_can_set_exception`,
// which emits the guard that reads it.
pub(crate) fn emit_buffer_get_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::BufferGet { base, index } = expr else {
        unreachable!()
    };
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let Scalar::MemoryView(base_ptr) = base_scalar else {
        panic!(
            "pycc_codegen: internal error: a buffer element load's base did not evaluate \
             to a memoryview"
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
    let element = builder
        .build_call(
            rt.buffer_f64_get,
            &[base_ptr.into(), raw_index.into()],
            "buffer_get",
        )
        .expect("build_call should not fail for a declared runtime function")
        .try_as_basic_value()
        .expect_basic("pycc_rt_buffer_f64_get returns a non-void f64")
        .into_float_value();
    Scalar::Float(element)
}

// #1116: `len(b)`. One call and no arithmetic, exactly as the load
// above -- and no D-173 exception check after it, because
// `pycc_rt_buffer_len` cannot raise (`expression_can_set_exception`
// answers `false` for this node, so no guard block is emitted).
pub(crate) fn emit_buffer_len_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::BufferLen { base } = expr else {
        unreachable!()
    };
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let Scalar::MemoryView(base_ptr) = base_scalar else {
        panic!(
            "pycc_codegen: internal error: a buffer length read's base did not evaluate \
             to a memoryview"
        )
    };
    let raw_len = builder
        .build_call(rt.buffer_len, &[base_ptr.into()], "buffer_len")
        .expect("build_call should not fail for a declared runtime function")
        .try_as_basic_value()
        .expect_basic("pycc_rt_buffer_len returns a non-void i64")
        .into_int_value();
    // D-141: the raw element count becomes a user-visible `Ty::Int`
    // expression value here, so it is tagged exactly as the scalar
    // `len` arm tags `pycc_rt_list_len`'s own raw count.
    Scalar::Int(raw_i64_to_tagged_int(context, builder, raw_len))
}

pub(crate) fn emit_buffer_alloc_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::BufferAlloc { len } = expr else {
        unreachable!()
    };
    let len_scalar = emit_expr(context, builder, module, rt, user_functions, locals, len);
    let encoded_len = to_numeric_encoded_int(context, builder, len_scalar);
    let raw_len = build_buffer_alloc_untag_len(builder, rt, encoded_len, "buffer_untag_alloc_len");
    // #1166 round 11, finding 1: retire the length's own birth
    // reference here, unconditionally, before anything below can
    // branch away.
    //
    // `n + 1` with `n == 2 ** 62 - 1` evaluates to a freshly owned
    // `BigIntObj`, and `to_numeric_encoded_int` passes that pointer
    // through unchanged. Every edge out of this arm that is reached
    // *after* the decode either abandons the value or consumes it:
    //
    //   * the decoder's own `OverflowError` on a bigint or malformed
    //     word -- the reviewer's case, and the only one a real
    //     program reaches with a live bigint, since promotion is the
    //     only way a bigint length exists here at all;
    //   * an exception already pending when this arm was reached,
    //     which the guard below short-circuits on;
    //   * the allocator's own negative-length `ValueError` and
    //     unreservable-length `RuntimeError`, both raised after
    //     `raw_len` is computed;
    //   * and the ordinary fallthrough.
    //
    // Retiring the reference *before* the guard is what covers the
    // first two as well as the last two: a release emitted after
    // `guard_statement_effects` would be skipped by exactly the
    // edges that leak. Repeatedly catching the `OverflowError`
    // leaked one `BigIntObj` per call in the host process before
    // this call existed.
    //
    // Unconditional rather than exception-edge-only, and a bare
    // release rather than a `push_pending_int_release_if_temporary`:
    // `raw_len` (or the decoder's sentinel `0`) is the only thing
    // read below, so the word is dead on *every* edge -- and
    // `rt.exceptions.pending_int_releases` is documented to carry an
    // *enclosing* node's operand, "never this call site's own
    // operand" (see `guard_statement_effects` and each push site).
    // Pushing this arm's own operand there would have the unwind
    // block release a word this arm must also retire on the normal
    // path, which is a double release rather than a leak.
    //
    // `release_if_int_temporary`'s own classification keeps a
    // borrowed length (`a = ndarray(n)`, where `n` names the value)
    // untouched, so only a birth reference is retired.
    release_if_int_temporary(context, builder, rt, len, encoded_len);
    // #1166 review finding F5: check the pending state *before*
    // allocating, not only after.
    //
    // `emit_expr`'s own guard runs after `emit_expr_unchecked`
    // returns, so an exception already pending when this arm is
    // reached -- set by an operation `expression_can_set_exception`
    // classifies `false` and therefore leaves unguarded, such as the
    // `MirExpr::ListPop` in `a = ndarray(xs.pop())` -- allowed the
    // allocation to succeed and *then* branched to the handler,
    // before `MirStmt::Assign` could store the pointer into the
    // frame's owned slot. The slot kept its entry null, so neither
    // the unwind nor the epilogue freed the view: one leaked buffer
    // per call, which `pycc_rt_buffer_live_views` counts.
    //
    // Guarding here instead short-circuits to the same exception
    // target without allocating, which is what makes the arm's
    // "a refused allocation leaves nothing for the unwind path to
    // release" claim hold for a *stale* pending exception as well as
    // for the allocator's own negative-length raise. It is also what
    // makes `build_buffer_alloc_untag_len` above safe: that decoder's
    // bigint refusal sets the pending state and returns the sentinel
    // `0`, and `0` is a *valid* length, so without this guard the
    // allocator would succeed on it and produce a zero-length view
    // that `MirStmt::Assign` never gets to store -- the same leak
    // shape as the stale-exception case above, reached through the
    // decoder instead. `decode_inline_or_raise`'s own contract
    // requires exactly this: return the sentinel before reaching any
    // further `raise_builtin`, since `raise_builtin` installs
    // unconditionally and would otherwise report over this raise.
    guard_statement_effects(context, builder, rt);
    let view = builder
        .build_call(rt.buffer_f64_alloc, &[raw_len.into()], "buffer_alloc")
        .expect("build_call should not fail for a declared runtime function")
        .try_as_basic_value()
        .expect_basic("pycc_rt_buffer_f64_alloc returns a non-void pointer")
        .into_pointer_value();
    // The identical `Scalar::MemoryView` a `memoryview` parameter
    // carries (see the parameter's own binding site), which is what
    // lets `BufferGet`, `BufferLen` and `MirStmt::BufferSet` operate
    // on an owned buffer with no arm of their own.
    Scalar::MemoryView(view)
}
