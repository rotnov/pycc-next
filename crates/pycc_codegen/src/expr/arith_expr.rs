//! Arithmetic, comparison and boolean expression lowering.

use super::*;
use pycc_mir::Ty;

pub(crate) fn emit_bin_op_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::BinOp {
        op,
        left,
        right,
        ty,
    } = expr
    else {
        unreachable!()
    };
    // This inkwell version's `try_as_basic_value()` returns its own
    // `ValueKind` enum (not `either::Either` as in older inkwell
    // releases the task brief's original code was written against
    // -- ".left()" doesn't exist on this type); `.expect_basic(msg)`
    // is the direct equivalent, panicking with `msg` if the callee
    // turned out to be void instead of returning a value.
    let l = emit_expr(context, builder, module, rt, user_functions, locals, left);
    // #638 (D-208): `l` may already be an owned, not-yet-released
    // `Ty::Int` birth reference (e.g. `x + x` promoting to a fresh
    // heap word). Protect it across `right`'s evaluation, which may
    // recurse into an exception-settable node and branch away
    // before this arm's own `release_if_int_temporary(left, l)`
    // call below is ever reached. The pop on the fallthrough path
    // immediately precedes that unchanged release call, so at most
    // one of {exception-edge release, fallthrough release} ever
    // executes for this word.
    let pending_l = push_pending_int_release_if_scalar_temporary(rt, left, &l);
    // A fresh counted left operand is released if `right` raises.
    let mark = heap_rc::pending_mark(rt);
    heap_rc::hold_temporary(rt, left, &l);
    let r = emit_expr(context, builder, module, rt, user_functions, locals, right);
    heap_rc::truncate_pending(rt, mark);
    pop_pending_int_release(rt, pending_l);
    if let Ty::List(_) = ty {
        let result = list::emit_list_binop(context, builder, rt, *op, l, r);
        release_scalar_if_temporary(context, builder, rt, left, &l);
        release_scalar_if_temporary(context, builder, rt, right, &r);
        return result;
    }
    match ty {
        Ty::Int => {
            // `to_numeric_encoded_int` promotes a `bool` operand instead of
            // rejecting it -- Python's `bool` is an `int` subtype
            // (e.g. `True + 1 == 2`), a case `pycc_types` already
            // legitimately types `Ty::Int` (see its own
            // `a_binop_treats_bool_as_int` test); this is normal
            // Python arithmetic-promotion semantics being handled,
            // not an internal invariant violation (this codegen's
            // own earlier, Task 3-era version mislabeled the
            // now-removed rejection of exactly this case as an
            // "internal error" -- see this task's own
            // `adding_a_bool_left_operand_to_an_int_promotes_
            // bool_to_int` test).
            // `&`, `|` and `^` keep D-141's bool markers instead
            // (#1210), which `binop::encode_int_operand` decides.
            let l = binop::encode_int_operand(context, builder, *op, l);
            let r = binop::encode_int_operand(context, builder, *op, r);
            // Inline smallint fast path, runtime call otherwise
            // (`int_fast`).
            // #146 Part 2 (D-181): both operands are dead after the
            // operation, so their birth references go to
            // `emit_int_binop`, which releases them on its runtime
            // path (a smallint fast path owns nothing). No runtime
            // `int` function returns an operand's own word, so the
            // release cannot free the result
            // (`an_int_operation_never_returns_an_operand_s_own_word`).
            let owned: Vec<_> = [
                bigint_rc::int_temporary_word(left, l),
                bigint_rc::int_temporary_word(right, r),
            ]
            .into_iter()
            .flatten()
            .collect();
            let result = int_fast::emit_int_binop(context, builder, rt, *op, l, r, &owned);
            Scalar::Int(result)
        }
        // `int / int` is exact-then-rounded at any magnitude, not
        // two conversions and a float division (which overflows for
        // `10**400 / 10**399` and double-rounds past 2**53).
        Ty::Float
            if *op == pycc_mir::BinOpKind::Div
                && matches!(left.ty(), Ty::Int | Ty::Bool)
                && matches!(right.ty(), Ty::Int | Ty::Bool) =>
        {
            let lw = to_numeric_encoded_int(context, builder, l);
            let rw = to_numeric_encoded_int(context, builder, r);
            let quotient = builder
                .build_call(rt.int_truediv, &[lw.into(), rw.into()], "int_truediv")
                .expect("build_call should not fail for int true division")
                .try_as_basic_value()
                .expect_basic("pycc_rt_int_truediv returns f64")
                .into_float_value();
            release_scalar_if_temporary(context, builder, rt, left, &l);
            release_scalar_if_temporary(context, builder, rt, right, &r);
            Scalar::Float(quotient)
        }
        Ty::Float => {
            // `float_fast`: each `int` operand's conversion, `/` and
            // `**` guard the pending exception on their own cold
            // paths (see `expression_can_set_exception`).
            let l = float_fast::operand(context, builder, rt, l);
            let r = float_fast::operand(context, builder, rt, r);
            match op {
                pycc_mir::BinOpKind::Add => Scalar::Float(
                    builder
                        .build_float_add(l, r, "fadd")
                        .expect("build_float_add should not fail for two f64 operands"),
                ),
                pycc_mir::BinOpKind::Sub => Scalar::Float(
                    builder
                        .build_float_sub(l, r, "fsub")
                        .expect("build_float_sub should not fail for two f64 operands"),
                ),
                pycc_mir::BinOpKind::Mul => Scalar::Float(
                    builder
                        .build_float_mul(l, r, "fmul")
                        .expect("build_float_mul should not fail for two f64 operands"),
                ),
                pycc_mir::BinOpKind::Div => {
                    Scalar::Float(float_fast::emit_div(context, builder, rt, l, r))
                }
                // Each arm binds its own `rt_fn` directly (rather
                // than a shared `FloorDiv | Mod | Pow` arm re-matching
                // `op` to pick one) so there is no redundant,
                // provably unreachable `_` fallback arm left over --
                // this outer `match op` already guarantees exactly
                // one of these three, and Rust's own exhaustiveness
                // checker (not a defensive catch-all) is what proves
                // that here (same reasoning as this file's other
                // documented dead-code removals, e.g.
                // `emit_expr`'s `Name` arm above).
                pycc_mir::BinOpKind::FloorDiv => {
                    Scalar::Float(build_float_rt_binop(builder, rt.float_floordiv, l, r))
                }
                pycc_mir::BinOpKind::Mod => {
                    Scalar::Float(build_float_rt_binop(builder, rt.float_floormod, l, r))
                }
                pycc_mir::BinOpKind::Pow => {
                    let exponent_literal = match right.as_ref() {
                        MirExpr::FloatLiteral(value) => Some(*value),
                        _ => None,
                    };
                    Scalar::Float(float_fast::emit_pow(
                        context,
                        builder,
                        rt,
                        l,
                        r,
                        exponent_literal,
                    ))
                }
                other => unreachable!("pycc_types rejects a float `{other:?}` (T0021)"),
            }
        }
        // #1210: `&`, `|` and `^` over two `bool` operands.
        Ty::Bool => binop::emit_bool_bitwise(builder, *op, l, r),
        Ty::Str => {
            // #575 (Part 2 of #123): string repetition. `pycc_types`
            // accepts `str * int` / `int * str` (with `bool` as the
            // count, since `bool <: int`) and
            // `pycc_mir::binop_result_ty` types it `str`; this arm
            // now emits it rather than stopping at Part 1's named
            // D-072 boundary. MIR needs no repetition node of its
            // own: `BinOp { op: Mul, ty: Ty::Str }` *is* the
            // repetition, and this `match ty` is what distinguishes
            // it from numeric multiplication.
            //
            // Handled *before* the two operand destructures below on
            // purpose: the count evaluates to a `Scalar::Int`/
            // `Scalar::Bool`, so leaving it to fall through would
            // report the destructures' "internal error" message.
            if *op == pycc_mir::BinOpKind::Mul {
                // Both operand orders land here -- whichever side is
                // the `str` becomes the operand, the other the count.
                //
                // Review note: a `str * str` pair (unreachable from
                // any real pipeline, since `pycc_types` rejects it
                // with `T0021`) matches the first arm and then fails
                // inside `to_numeric_encoded_int` with its own
                // "expected an int-or-bool operand, got str" message
                // rather than the `str {op} str` message below. That
                // shadowing is accepted, exactly as Part 1 accepted
                // it, rather than worked around for a shape no
                // type-checked program can produce.
                let (operand, count) = match (l, r) {
                    (Scalar::Str(operand), count) => (operand, count),
                    (count, Scalar::Str(operand)) => (operand, count),
                    _ => {
                        panic!("pycc_codegen: internal error: a str-result `*` had no str operand")
                    }
                };
                // `to_numeric_encoded_int` promotes a `bool` count to
                // the same D-141 encoding an `int` count already
                // carries (`"ab" * True` is `"ab"`), and
                // `build_untag_checked` decodes it to the raw counter
                // `pycc_rt_str_repeat` expects -- keeping bigint
                // rejection in D-141's single runtime-owned
                // classifier instead of duplicating it here.
                let encoded = to_numeric_encoded_int(context, builder, count);
                let raw = build_untag_checked(
                    context,
                    builder,
                    rt,
                    UntagUse::Size,
                    encoded,
                    "str_repeat_count",
                );
                let result = builder
                    .build_call(rt.str_repeat, &[operand.into(), raw.into()], "str_repeat")
                    .expect("build_call should not fail for a well-formed repetition")
                    .try_as_basic_value()
                    .expect_basic("pycc_rt_str_repeat returns a non-void pointer");
                // The repetition copies its operand's bytes, so both
                // operands are dead once it has run.
                release_scalar_if_temporary(context, builder, rt, left, &l);
                release_scalar_if_temporary(context, builder, rt, right, &r);
                return Scalar::Str(result.into_pointer_value());
            }
            let Scalar::Str(l) = l else {
                panic!("pycc_codegen: internal error: str BinOp operand did not evaluate to str")
            };
            let Scalar::Str(r) = r else {
                panic!("pycc_codegen: internal error: str BinOp operand did not evaluate to str")
            };
            if *op != pycc_mir::BinOpKind::Add {
                panic!(
                    "pycc_codegen: `str {op:?} str` is not supported yet (only concatenation is)"
                );
            }
            let result = builder
                .build_call(rt.str_concat, &[l.into(), r.into()], "str_concat")
                .expect("build_call should not fail for a well-formed concatenation")
                .try_as_basic_value()
                .expect_basic("pycc_rt_str_concat returns a non-void pointer");
            // `pycc_rt_str_concat` copies both operands' bytes into
            // a new object and keeps neither.
            release_scalar_if_temporary(context, builder, rt, left, &Scalar::Str(l));
            release_scalar_if_temporary(context, builder, rt, right, &Scalar::Str(r));
            Scalar::Str(result.into_pointer_value())
        }
        // A list result returned above (`list::emit_list_binop`);
        // no other container type supports a `BinOpKind`. This arm
        // names the container type in the message rather than
        // falling to the generic catch-all below.
        Ty::List(_) | Ty::Dict(..) | Ty::Set(_) | Ty::FrozenSet(_) | Ty::Tuple(_) => {
            panic!(
                "pycc_codegen: binary operators are not supported on {} yet",
                ty.name()
            )
        }
        other => panic!("pycc_codegen: a `{other:?}`-result BinOp is not supported yet"),
    }
}

pub(crate) fn emit_compare_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::Compare {
        op, left, right, ..
    } = expr
    else {
        unreachable!()
    };
    let l = emit_expr(context, builder, module, rt, user_functions, locals, left);
    // #638 (D-208): same protection as `BinOp`'s `Ty::Int` arm --
    // `l` must survive `right`'s evaluation intact so its release
    // below is guaranteed to see it, even when `right`'s evaluation
    // branches away on an exception.
    let pending_l = push_pending_int_release_if_scalar_temporary(rt, left, &l);
    let mark = heap_rc::pending_mark(rt);
    heap_rc::hold_temporary(rt, left, &l);
    let r = emit_expr(context, builder, module, rt, user_functions, locals, right);
    heap_rc::truncate_pending(rt, mark);
    pop_pending_int_release(rt, pending_l);
    let right_ty = right.ty();
    if matches!(op, pycc_mir::CmpOpKind::In | pycc_mir::CmpOpKind::NotIn)
        && containers::has_native_contains(&right_ty)
    {
        let negate = *op == pycc_mir::CmpOpKind::NotIn;
        let found = containers::emit_contains(context, builder, rt, &right_ty, l, r, negate);
        release_scalar_if_temporary(context, builder, rt, left, &l);
        release_scalar_if_temporary(context, builder, rt, right, &r);
        return found;
    }
    let as_bool = compare::emit_compare_values(context, builder, rt, *op, left, l, right, r);
    // #146 Part 2 (D-181): no comparison retains either operand, so
    // both are dead the moment the comparison has been made. #1212
    // moved this out of the int-only branch, which also releases an
    // `int` temporary compared against a `float`
    // (`pycc_rt_int_to_float` reads its operand and consumes
    // nothing).
    release_scalar_if_temporary(context, builder, rt, left, &l);
    release_scalar_if_temporary(context, builder, rt, right, &r);
    Scalar::Bool(as_bool)
}

// #1212 (Part 4 of #1018): chained comparisons. See
// `compare_chain.rs`.
pub(crate) fn emit_compare_chain_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::CompareChain { first, links } = expr else {
        unreachable!()
    };
    compare_chain::emit_compare_chain(
        &boolop::Emitter {
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
        },
        first,
        links,
    )
}

// #604 (Part 3 of #573): `not x`. Reuses `truthy`, the exact same
// helper an `if`/`while` condition's own test already calls (see
// `MirStmt::If` above), then inverts the resulting `i1` -- so a
// `not` over any operand `truthy` can classify (`bool`/`int`/
// `float`/`str`/`Optional`, and a CPython object since Part 10 of
// #1371; `pycc_types::unop::unary_result_type` rejects every other
// operand before this ever runs) gets
// `if`/`while`'s exact truthiness semantics for free, including
// the `float`'s `UNE`-not-`ONE` NaN handling and `Optional`'s
// present/payload AND. Every other truthy-call site releases any
// int temporary the operand produced *after* `truthy` reads it
// (#146 Part 2, D-181) -- this one follows the identical sequence.
// #1211 (Part 3 of #1018): `and`/`or`. See `boolop.rs`.
pub(crate) fn emit_bool_op_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::BoolOp {
        op,
        left,
        right,
        ty,
        truth_only,
    } = expr
    else {
        unreachable!()
    };
    boolop::emit_bool_op(
        &boolop::Emitter {
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
        },
        *op,
        left,
        right,
        ty,
        *truth_only,
    )
}

// #1395: `body if test else orelse`. See `if_exp.rs`.
pub(crate) fn emit_if_exp_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::IfExp {
        test,
        body,
        orelse,
        ty,
    } = expr
    else {
        unreachable!()
    };
    if_exp::emit_if_exp(
        &boolop::Emitter {
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
        },
        test,
        body,
        orelse,
        ty,
    )
}

pub(crate) fn emit_not_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::Not(operand) = expr else {
        unreachable!()
    };
    let operand_scalar = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        operand,
    );
    let held = object_release::hold(context, module, rt, operand, &operand_scalar);
    let truthy_cond = truthy(context, builder, module, rt, operand_scalar);
    held.release(builder, rt);
    release_scalar_if_temporary(context, builder, rt, operand, &operand_scalar);
    let inverted = builder
        .build_not(truthy_cond, "not_truthy")
        .expect("build_not should not fail inverting a well-formed i1");
    let as_bool = builder
        .build_int_z_extend(inverted, context.i8_type(), "bool_from_not")
        .expect("build_int_z_extend should not fail widening i1 to i8");
    Scalar::Bool(as_bool)
}
