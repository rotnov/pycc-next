//! Inlining of tiny leaf functions.
//!
//! A function whose whole body is `return <arithmetic over its parameters
//! and literals>` -- `def a(i, j): return 1.0 / ((i + j) * (i + j + 1) // 2
//! + i + 1)` -- is a few instructions on its fast paths, but every operation
//! carries a cold runtime fallback (bigints, a zero divisor), and LLVM's
//! inliner counts those calls, so it leaves the function out of line in the
//! very loops that call it most. Such a function gets `alwaysinline`. It
//! calls nothing, so inlining cannot recurse, and its size is bounded by
//! [`MAX_NODES`].

use pycc_mir::{MirExpr, MirStmt};

/// The largest expression (in nodes) a leaf function may return.
const MAX_NODES: usize = 32;

/// Whether a function with body `body` should be inlined everywhere.
pub(super) fn is_tiny_leaf(body: &[MirStmt]) -> bool {
    let body: Vec<&MirStmt> = body.iter().filter(|stmt| !matches!(stmt, MirStmt::Pos(_))).collect();
    let [MirStmt::Return(Some(value))] = body.as_slice() else {
        return false;
    };
    let mut budget = MAX_NODES;
    is_arithmetic(value, &mut budget)
}

/// Whether `expr` is arithmetic over names and literals of at most
/// `budget` nodes (decremented as it is walked).
fn is_arithmetic(expr: &MirExpr, budget: &mut usize) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    match expr {
        MirExpr::Name { .. }
        | MirExpr::IntLiteral(_)
        | MirExpr::FloatLiteral(_)
        | MirExpr::BoolLiteral(_) => true,
        MirExpr::IntBoundary(inner) => is_arithmetic(inner, budget),
        MirExpr::BinOp { left, right, .. } => {
            is_arithmetic(left, budget) && is_arithmetic(right, budget)
        }
        _ => false,
    }
}

/// Marks `function` `alwaysinline`.
pub(super) fn mark<'ctx>(
    context: &'ctx inkwell::context::Context,
    function: inkwell::values::FunctionValue<'ctx>,
) {
    let kind = inkwell::attributes::Attribute::get_named_enum_kind_id("alwaysinline");
    function.add_attribute(
        inkwell::attributes::AttributeLoc::Function,
        context.create_enum_attribute(kind, 0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use pycc_mir::{BinOpKind, Ty};

    fn name(n: &str) -> MirExpr {
        MirExpr::Name {
            name: n.to_string(),
            ty: Ty::Int,
        }
    }

    fn add(l: MirExpr, r: MirExpr) -> MirExpr {
        MirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(l),
            right: Box::new(r),
            ty: Ty::Int,
        }
    }

    #[test]
    fn a_single_arithmetic_return_is_a_tiny_leaf() {
        let body = vec![MirStmt::Return(Some(add(name("i"), MirExpr::IntLiteral(1))))];
        assert!(is_tiny_leaf(&body));
    }

    #[test]
    fn calls_statements_and_large_expressions_are_not() {
        let call = MirExpr::Call {
            callee: "f".to_string(),
            args: Vec::new(),
            ty: Ty::Int,
        };
        assert!(!is_tiny_leaf(&[MirStmt::Return(Some(call))]));
        assert!(!is_tiny_leaf(&[MirStmt::Return(None)]));
        assert!(!is_tiny_leaf(&[
            MirStmt::NoOp,
            MirStmt::Return(Some(name("i")))
        ]));
        let mut big = name("i");
        for _ in 0..MAX_NODES {
            big = add(big, MirExpr::IntLiteral(1));
        }
        assert!(!is_tiny_leaf(&[MirStmt::Return(Some(big))]));
    }
}
