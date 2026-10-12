//! One smallint fast path for a whole `int` expression tree.
//!
//! `int_fast` gives each operator its own fast path, whose result joins the
//! runtime path's (possibly bigint) result in a phi -- so the next operator
//! tests the tag again, and LLVM cannot prove it away. A tree such as
//! `(i + j) * (i + j + 1) // 2 + i + 1` paid one tag test, one join and one
//! retag per operator.
//!
//! Here a tree of `+`, `-`, `*` and `//`/`%` by a positive literal over `int`
//! names and smallint literals is evaluated by testing every leaf's tag once,
//! untagging, computing the whole tree on raw `i64`s with overflow checks,
//! and tagging the result once (if it fits the tagged range). Any non-small
//! leaf or any overflow takes the old per-operator emission over the same
//! leaf words. The leaves are read before any operator runs: a name read
//! has no side effect, and these operators neither raise nor run code, so
//! the order cannot be observed.

use crate::rt_fns::RtFns;
use crate::{Scalar, StorageSlot, UserFunction};
use inkwell::IntPredicate;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::values::IntValue;
use pycc_mir::{BinOpKind, MirExpr, Ty};
use std::collections::HashMap;

/// Largest literal magnitude a leaf may have (it must be a smallint, and a
/// raw product of two leaves must not hide an overflow).
const MAX_LEAF_LITERAL: i64 = 1 << 61;

/// Whether `expr` is a tree node this module handles (see the module doc).
fn is_node(expr: &MirExpr) -> bool {
    let MirExpr::BinOp {
        op,
        left,
        right,
        ty: Ty::Int,
    } = expr
    else {
        return false;
    };
    if left.ty() != Ty::Int || right.ty() != Ty::Int {
        return false;
    }
    match op {
        BinOpKind::Add | BinOpKind::Sub | BinOpKind::Mul => is_operand(left) && is_operand(right),
        BinOpKind::FloorDiv | BinOpKind::Mod => {
            is_operand(left) && matches!(**right, MirExpr::IntLiteral(k) if k > 0 && k < MAX_LEAF_LITERAL)
        }
        _ => false,
    }
}

fn is_leaf(expr: &MirExpr) -> bool {
    match expr {
        MirExpr::Name { ty: Ty::Int, .. } => true,
        MirExpr::IntLiteral(n) => n.unsigned_abs() < MAX_LEAF_LITERAL as u64,
        _ => false,
    }
}

fn is_operand(expr: &MirExpr) -> bool {
    is_leaf(expr) || is_node(expr)
}

fn node_count(expr: &MirExpr) -> usize {
    match expr {
        MirExpr::BinOp { left, right, .. } => 1 + node_count(left) + node_count(right),
        _ => 0,
    }
}

/// Whether `expr` is a tree of at least two operators: one operator gains
/// nothing over `int_fast`.
pub(crate) fn applies(expr: &MirExpr) -> bool {
    is_node(expr) && node_count(expr) >= 2
}

/// Emits a tree [`applies`] accepts and returns its encoded result.
pub(crate) fn emit<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> IntValue<'ctx> {
    // Read every distinct name once, in source order.
    let mut names: Vec<(&str, IntValue<'ctx>)> = Vec::new();
    collect_names(expr, &mut |leaf| {
        let MirExpr::Name { name, .. } = leaf else {
            return;
        };
        if names.iter().any(|(seen, _)| *seen == name.as_str()) {
            return;
        }
        let word = match crate::emit_expr(context, builder, module, rt, user_functions, locals, leaf) {
            Scalar::Int(word) => word,
            other => crate::to_numeric_encoded_int(context, builder, other),
        };
        names.push((name.as_str(), word));
    });
    let word_of = |name: &str| {
        names
            .iter()
            .find(|(seen, _)| *seen == name)
            .map(|(_, word)| *word)
            .expect("every name leaf was read")
    };
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("an expression is emitted inside a function");
    let fast_bb = context.append_basic_block(function, "int_tree_fast");
    let slow_bb = context.append_basic_block(function, "int_tree_slow");
    let join_bb = context.append_basic_block(function, "int_tree_join");
    let i64_type = context.i64_type();
    let one = i64_type.const_int(1, false);
    let words: Vec<IntValue<'ctx>> = names.iter().map(|(_, word)| *word).collect();
    if words.is_empty() {
        builder.build_unconditional_branch(fast_bb).expect("branch");
    } else {
        let small = crate::int_fast::all_small(context, builder, &words);
        crate::int_fast::likely_branch(context, builder, small, fast_bb, slow_bb);
    }

    // Fast path: raw arithmetic, every overflow collected into one flag.
    builder.position_at_end(fast_bb);
    let mut overflow = context.bool_type().const_zero();
    let raw = fast(context, builder, rt, expr, &word_of, &mut overflow);
    let (doubled, retag_overflow) = crate::int_fast::with_overflow(builder, rt.sadd_overflow, raw, raw);
    let overflow = builder.build_or(overflow, retag_overflow, "int_tree_ov").expect("or");
    let tagged = builder.build_or(doubled, one, "int_tree_tag").expect("or");
    let fast_end = builder.get_insert_block().expect("positioned");
    let branch = builder
        .build_conditional_branch(overflow, slow_bb, join_bb)
        .expect("branch");
    crate::int_fast::weight_branch(context, branch, false);

    // Slow path: the per-operator emission over the same leaf words.
    builder.position_at_end(slow_bb);
    let (slow_word, _) = slow(context, builder, rt, expr, &word_of);
    let slow_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join_bb).expect("branch");

    builder.position_at_end(join_bb);
    let phi = builder.build_phi(i64_type, "int_tree").expect("phi");
    phi.add_incoming(&[(&tagged, fast_end), (&slow_word, slow_end)]);
    phi.as_basic_value().into_int_value()
}

fn collect_names<'e>(expr: &'e MirExpr, visit: &mut impl FnMut(&'e MirExpr)) {
    match expr {
        MirExpr::BinOp { left, right, .. } => {
            collect_names(left, visit);
            collect_names(right, visit);
        }
        leaf => visit(leaf),
    }
}

/// The raw (untagged) value of `expr` on the fast path; ORs every overflow
/// into `overflow`.
fn fast<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    expr: &MirExpr,
    word_of: &impl Fn(&str) -> IntValue<'ctx>,
    overflow: &mut IntValue<'ctx>,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let zero = i64_type.const_zero();
    match expr {
        MirExpr::IntLiteral(n) => i64_type.const_int(*n as u64, true),
        MirExpr::Name { name, .. } => builder
            .build_right_shift(word_of(name), i64_type.const_int(1, false), true, "int_tree_leaf")
            .expect("ashr"),
        MirExpr::BinOp {
            op, left, right, ..
        } => {
            let l = fast(context, builder, rt, left, word_of, overflow);
            match op {
                BinOpKind::FloorDiv | BinOpKind::Mod => {
                    let MirExpr::IntLiteral(k) = **right else {
                        unreachable!("a tree divides by a literal")
                    };
                    let k_value = i64_type.const_int(k as u64, true);
                    if *op == BinOpKind::FloorDiv && (k as u64).is_power_of_two() {
                        let shift = i64_type.const_int(u64::from(k.trailing_zeros()), false);
                        return builder
                            .build_right_shift(l, shift, true, "int_tree_shr")
                            .expect("ashr");
                    }
                    // `k > 0`: Python floors, so a negative remainder moves
                    // the quotient down by one and the remainder up by `k`.
                    let q = builder.build_int_signed_div(l, k_value, "int_tree_q").expect("sdiv");
                    let rem = builder.build_int_signed_rem(l, k_value, "int_tree_r").expect("srem");
                    let negative = builder
                        .build_int_compare(IntPredicate::SLT, rem, zero, "int_tree_rneg")
                        .expect("icmp");
                    if *op == BinOpKind::FloorDiv {
                        let adj = builder
                            .build_int_z_extend(negative, i64_type, "int_tree_adj")
                            .expect("zext");
                        builder.build_int_sub(q, adj, "int_tree_floor").expect("sub")
                    } else {
                        let fixed = builder.build_int_add(rem, k_value, "int_tree_rfix").expect("add");
                        builder
                            .build_select(negative, fixed, rem, "int_tree_mod")
                            .expect("select")
                            .into_int_value()
                    }
                }
                _ => {
                    let r = fast(context, builder, rt, right, word_of, overflow);
                    let intrinsic = match op {
                        BinOpKind::Add => rt.sadd_overflow,
                        BinOpKind::Sub => rt.ssub_overflow,
                        _ => rt.smul_overflow,
                    };
                    let (value, flag) = crate::int_fast::with_overflow(builder, intrinsic, l, r);
                    *overflow = builder.build_or(*overflow, flag, "int_tree_ovacc").expect("or");
                    value
                }
            }
        }
        _ => unreachable!("only names, literals and operators are tree nodes"),
    }
}

/// The encoded value of `expr` through `int_fast`'s per-operator emission,
/// and whether the caller owns it (an operator's fresh result).
fn slow<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    expr: &MirExpr,
    word_of: &impl Fn(&str) -> IntValue<'ctx>,
) -> (IntValue<'ctx>, bool) {
    match expr {
        MirExpr::IntLiteral(n) => (crate::tag_smallint_const(context, *n), false),
        MirExpr::Name { name, .. } => (word_of(name), false),
        MirExpr::BinOp {
            op, left, right, ..
        } => {
            let (l, l_owned) = slow(context, builder, rt, left, word_of);
            let (r, r_owned) = slow(context, builder, rt, right, word_of);
            let owned: Vec<IntValue<'ctx>> = [(l, l_owned), (r, r_owned)]
                .into_iter()
                .filter_map(|(word, owned)| owned.then_some(word))
                .collect();
            (
                crate::int_fast::emit_int_binop(context, builder, rt, *op, l, r, &owned),
                true,
            )
        }
        _ => unreachable!("only names, literals and operators are tree nodes"),
    }
}
