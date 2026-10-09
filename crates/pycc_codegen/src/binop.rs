//! The bitwise and shift parts of `emit_expr`'s `BinOp` arm (#1210).
//!
//! Two choices live here. [`encode_int_operand`] picks how an `int`-result
//! operand is encoded: `&`, `|` and `^` keep D-141's bool markers, because
//! two `bool` objects combine to a `bool` even when the static type only
//! says `int` (`x: int = True; x & True` is `True`), while every other
//! operator, the shifts included, promotes a `bool` to the smallint `0` or
//! `1` (`True << 1` is `2`). [`emit_bool_bitwise`] emits `&`, `|` and `^`
//! over two statically `bool` operands directly on their `i8` carriers.

use super::{Scalar, to_encoded_int, to_numeric_encoded_int};
use inkwell::context::Context;
use inkwell::values::IntValue;
use pycc_mir::BinOpKind;

/// Whether `op` is one of the three operators that keep bool identity.
fn keeps_bool_identity(op: BinOpKind) -> bool {
    matches!(op, BinOpKind::BitAnd | BinOpKind::BitOr | BinOpKind::BitXor)
}

/// Encodes one operand of an `int`-result `BinOp` as the word the
/// `pycc_rt_int_*` function for `op` expects.
pub(super) fn encode_int_operand<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    op: BinOpKind,
    scalar: Scalar<'ctx>,
) -> IntValue<'ctx> {
    if keeps_bool_identity(op) {
        to_encoded_int(context, builder, scalar)
    } else {
        to_numeric_encoded_int(context, builder, scalar)
    }
}

/// Emits a `bool`-result `BinOp`: `&`, `|` or `^` over two `bool`
/// operands. `pycc_types` gives no other operator a `bool` result, so any
/// other shape is malformed MIR and panics.
pub(super) fn emit_bool_bitwise<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    op: BinOpKind,
    left: Scalar<'ctx>,
    right: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let (Scalar::Bool(l), Scalar::Bool(r)) = (left, right) else {
        panic!("pycc_codegen: internal error: a bool-result BinOp had a non-bool operand")
    };
    let value = match op {
        BinOpKind::BitAnd => builder.build_and(l, r, "bool_and"),
        BinOpKind::BitOr => builder.build_or(l, r, "bool_or"),
        BinOpKind::BitXor => builder.build_xor(l, r, "bool_xor"),
        other => panic!("pycc_codegen: internal error: a bool-result `{other:?}` BinOp"),
    };
    Scalar::Bool(value.expect("an i8 bitwise instruction should not fail to build"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pycc_mir::{MirExpr, MirItem, MirModule, MirStmt, Ty};

    /// Runs `body` with a builder positioned inside a fresh function.
    fn with_builder(body: impl FnOnce(&Context, &inkwell::builder::Builder<'_>)) {
        let context = Context::create();
        let module = context.create_module("binop");
        let function = module.add_function("f", context.void_type().fn_type(&[], false), None);
        let builder = context.create_builder();
        builder.position_at_end(context.append_basic_block(function, "entry"));
        body(&context, &builder);
    }

    fn constant_of(value: IntValue<'_>) -> u64 {
        value
            .get_zero_extended_constant()
            .expect("constant operands fold to a constant")
    }

    #[test]
    fn two_bool_constants_fold_through_each_operator() {
        with_builder(|context, builder| {
            let i8_type = context.i8_type();
            for (op, expected) in [
                (BinOpKind::BitAnd, 0),
                (BinOpKind::BitOr, 1),
                (BinOpKind::BitXor, 1),
            ] {
                let left = Scalar::Bool(i8_type.const_int(1, false));
                let right = Scalar::Bool(i8_type.const_int(0, false));
                let result = emit_bool_bitwise(builder, op, left, right);
                assert!(
                    matches!(result, Scalar::Bool(value) if constant_of(value) == expected),
                    "{op:?} should fold to the bool {expected}"
                );
            }
        });
    }

    /// `&`, `|` and `^` keep the `True` marker `6`; a shift promotes the
    /// same operand to the smallint `1`, encoded as `3`.
    #[test]
    fn only_the_logical_operators_keep_the_bool_marker() {
        with_builder(|context, builder| {
            let truth = || Scalar::Bool(context.i8_type().const_int(1, false));
            for (op, expected) in [
                (BinOpKind::BitAnd, 6),
                (BinOpKind::BitOr, 6),
                (BinOpKind::BitXor, 6),
                (BinOpKind::LShift, 3),
                (BinOpKind::RShift, 3),
                (BinOpKind::Add, 3),
            ] {
                let word = encode_int_operand(context, builder, op, truth());
                assert_eq!(constant_of(word), expected, "{op:?}");
            }
        });
    }

    #[test]
    #[should_panic(expected = "a bool-result BinOp had a non-bool operand")]
    fn a_non_bool_operand_is_malformed_mir() {
        with_builder(|context, builder| {
            let one = Scalar::Int(context.i64_type().const_int(3, false));
            let truth = Scalar::Bool(context.i8_type().const_int(1, false));
            emit_bool_bitwise(builder, BinOpKind::BitAnd, one, truth);
        });
    }

    fn binop(op: BinOpKind, left: MirExpr, right: MirExpr, ty: Ty) -> MirStmt {
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
                ty,
            }],
            ty: Ty::None,
        })
    }

    fn compile(label: &str, stmts: Vec<MirStmt>) -> Result<(), String> {
        let mir = MirModule {
            items: stmts.into_iter().map(MirItem::TopLevelStmt).collect(),
            class_defs: Vec::new(),
        };
        let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
        crate::compile_to_object(&mir, &dir.join("binop.o"), None, false)
    }

    /// Each new kind reaches its runtime function from the `int` arm, and
    /// `&`, `|` and `^` over two `bool` operands reach the `bool` arm.
    #[test]
    fn every_bitwise_and_shift_shape_compiles() {
        let mut stmts = Vec::new();
        for op in [
            BinOpKind::LShift,
            BinOpKind::RShift,
            BinOpKind::BitAnd,
            BinOpKind::BitOr,
            BinOpKind::BitXor,
        ] {
            let left = MirExpr::BoolLiteral(true);
            stmts.push(binop(op, left, MirExpr::IntLiteral(5), Ty::Int));
        }
        for op in [BinOpKind::BitAnd, BinOpKind::BitOr, BinOpKind::BitXor] {
            let (left, right) = (MirExpr::BoolLiteral(true), MirExpr::BoolLiteral(false));
            stmts.push(binop(op, left, right, Ty::Bool));
        }
        compile("binop_bitwise_shapes", stmts).expect("codegen should succeed");
    }

    #[test]
    #[should_panic(expected = "pycc_types rejects a float `BitXor` (T0021)")]
    fn a_float_bitwise_binop_is_malformed_mir() {
        let (left, right) = (MirExpr::FloatLiteral(1.0), MirExpr::FloatLiteral(2.0));
        let _ = compile(
            "binop_float_bitwise",
            vec![binop(BinOpKind::BitXor, left, right, Ty::Float)],
        );
    }

    #[test]
    #[should_panic(expected = "a bool-result `LShift` BinOp")]
    fn a_bool_result_shift_is_malformed_mir() {
        with_builder(|context, builder| {
            let truth = || Scalar::Bool(context.i8_type().const_int(1, false));
            emit_bool_bitwise(builder, BinOpKind::LShift, truth(), truth());
        });
    }
}
