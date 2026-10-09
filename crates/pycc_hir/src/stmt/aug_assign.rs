//! Augmented assignment (`target op= value`, #1209, Part 1 of #1018).
//!
//! `docs/TYPE_SYSTEM.md`'s "Augmented assignment" section is the canonical
//! statement of the invariant and of the soundness conditions this module
//! relies on. In short: an admitted statement is rewritten at the AST level
//! into the plain assignment `target = target op value` and handed back to
//! `lower_stmt`, so every check the `Stmt::Assign` arm runs (the #618 int
//! boundary, `super()`, the walrus-placement check, the property setter, the
//! memoryview store split) applies to it unchanged. The rewrite reads the
//! target twice, once to load and once to store, which is only observably
//! identical to CPython's single evaluation because the gates below admit a
//! container that is a bare name and an index that
//! [`is_unobservable`](crate::expr::unobservable::is_unobservable) accepts.
//! Everything else is refused here with its own `C0001`.

use crate::expr::bin_op_kind;
use crate::expr::unobservable::is_unobservable;
use crate::unsupported;
use pycc_ast::{Expr, ExprBinOp, ExprContext, Stmt, StmtAssign, StmtAugAssign};
use pycc_diag::Diagnostic;

/// Rewrites `aug` into the synthetic `Stmt::Assign` `target = target op
/// value`, or refuses it with a `C0001` naming the unsupported part. The
/// caller lowers the returned statement through `lower_stmt`'s ordinary
/// `Stmt::Assign` arm.
pub(super) fn desugar_aug_assign(aug: &StmtAugAssign) -> Result<Stmt, Diagnostic> {
    // The admitted operator set is exactly the plain binary operator set,
    // by construction: both paths ask the one `bin_op_kind` mapping.
    if bin_op_kind(aug.op).is_none() {
        return Err(unsupported(
            format!(
                "augmented assignment operator `{}=` is not supported yet",
                aug.op.as_str()
            ),
            aug.range,
        ));
    }
    let load = match aug.target.as_ref() {
        Expr::Name(name) => {
            let mut name = name.clone();
            name.ctx = ExprContext::Load;
            Expr::Name(name)
        }
        Expr::Attribute(attr) => {
            if !matches!(attr.value.as_ref(), Expr::Name(_)) {
                return Err(unsupported(
                    "augmented assignment to an attribute of a computed expression is not \
                     supported yet; bind the object to a name first",
                    aug.range,
                ));
            }
            let mut attr = attr.clone();
            attr.ctx = ExprContext::Load;
            Expr::Attribute(attr)
        }
        Expr::Subscript(subscript) => {
            if !matches!(subscript.value.as_ref(), Expr::Name(_)) {
                return Err(unsupported(
                    "augmented assignment to a subscript of a computed expression is not \
                     supported yet; bind the object to a name first",
                    aug.range,
                ));
            }
            if matches!(subscript.slice.as_ref(), Expr::Slice(_)) {
                return Err(unsupported(
                    "augmented assignment to a slice is not supported yet",
                    aug.range,
                ));
            }
            if !is_unobservable(&subscript.slice) {
                return Err(unsupported(
                    "augmented assignment with a computed index is not supported yet; bind \
                     the index to a name first",
                    aug.range,
                ));
            }
            let mut subscript = subscript.clone();
            subscript.ctx = ExprContext::Load;
            Expr::Subscript(subscript)
        }
        // Python's grammar admits only the three targets above for `op=`,
        // so the parser never produces another; a hand-built node still gets
        // the generic statement-kind refusal rather than a panic.
        _ => {
            return Err(unsupported(
                format!(
                    "statement kind not supported yet: {}",
                    pycc_ast::stmt_kind_name(&Stmt::AugAssign(aug.clone()))
                ),
                aug.range,
            ));
        }
    };
    Ok(Stmt::Assign(StmtAssign {
        node_index: Default::default(),
        range: aug.range,
        targets: vec![aug.target.as_ref().clone()],
        value: Box::new(Expr::BinOp(ExprBinOp {
            node_index: Default::default(),
            range: aug.range,
            left: Box::new(load),
            op: aug.op,
            right: aug.value.clone(),
        })),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pycc_ast::{ExprTuple, Operator};
    use pycc_diag::Span;

    /// Parses `source` (one augmented assignment) and returns its node.
    fn aug(source: &str) -> StmtAugAssign {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        module.body[0]
            .as_aug_assign_stmt()
            .expect("an augmented assignment")
            .clone()
    }

    fn refusal(source: &str) -> Diagnostic {
        desugar_aug_assign(&aug(source)).expect_err(source)
    }

    /// The one operator without a `BinOpKind` (`@`, since #1210 mapped the
    /// bitwise and shift operators) is refused in Python spelling, spanned on
    /// the whole statement.
    #[test]
    fn an_operator_with_no_binary_kind_is_refused_in_python_spelling() {
        let source = "x @= y";
        let diagnostic = refusal(source);
        assert_eq!(diagnostic.code, "C0001");
        assert_eq!(
            diagnostic.message,
            "augmented assignment operator `@=` is not supported yet"
        );
        assert_eq!(diagnostic.span, Some(Span::new(0, source.len() as u32)));
    }

    #[test]
    fn each_refused_target_shape_names_its_own_reason() {
        for (source, message) in [
            (
                "a.b.c += 1",
                "augmented assignment to an attribute of a computed expression is not supported \
                 yet; bind the object to a name first",
            ),
            (
                "f().x += 1",
                "augmented assignment to an attribute of a computed expression is not supported \
                 yet; bind the object to a name first",
            ),
            (
                "a.b[0] += 1",
                "augmented assignment to a subscript of a computed expression is not supported \
                 yet; bind the object to a name first",
            ),
            (
                "xs[1:3] += ys",
                "augmented assignment to a slice is not supported yet",
            ),
            (
                "d[f()] += 1",
                "augmented assignment with a computed index is not supported yet; bind the \
                 index to a name first",
            ),
            (
                "v[i + 1] += x",
                "augmented assignment with a computed index is not supported yet; bind the \
                 index to a name first",
            ),
        ] {
            let diagnostic = refusal(source);
            assert_eq!(diagnostic.code, "C0001", "{source}");
            assert_eq!(diagnostic.message, message, "{source}");
            assert_eq!(
                diagnostic.span,
                Some(Span::new(0, source.len() as u32)),
                "{source}"
            );
        }
    }

    /// A target the grammar never produces keeps the generic refusal.
    #[test]
    fn a_hand_built_tuple_target_gets_the_generic_statement_refusal() {
        let mut node = aug("x += 1");
        *node.target = Expr::Tuple(ExprTuple {
            node_index: Default::default(),
            range: Default::default(),
            elts: Vec::new(),
            ctx: ExprContext::Store,
            parenthesized: true,
        });
        let diagnostic = desugar_aug_assign(&node).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert_eq!(
            diagnostic.message,
            "statement kind not supported yet: an augmented assignment (`x += 1`)"
        );
    }

    /// Each admitted shape becomes `target = target op value`, with the
    /// target cloned into load position on the left operand.
    #[test]
    fn each_admitted_shape_desugars_to_the_plain_assignment() {
        for (source, op) in [
            ("x += 1", Operator::Add),
            ("self.n -= k", Operator::Sub),
            ("d[k] *= 2", Operator::Mult),
            ("d[\"a\"] /= 2.0", Operator::Div),
            ("v[0] //= 3", Operator::FloorDiv),
            ("v[-1] %= 3", Operator::Mod),
            ("t **= 2", Operator::Pow),
            ("x <<= 1", Operator::LShift),
            ("self.n >>= k", Operator::RShift),
            ("d[k] &= 3", Operator::BitAnd),
            ("x |= y", Operator::BitOr),
            ("v[0] ^= 1", Operator::BitXor),
        ] {
            let node = aug(source);
            let desugared = desugar_aug_assign(&node).expect(source);
            let assign = desugared.as_assign_stmt().expect(source);
            assert_eq!(assign.range, node.range, "{source}");
            assert_eq!(
                assign.targets,
                vec![node.target.as_ref().clone()],
                "{source}"
            );
            let bin_op = assign.value.as_bin_op_expr().expect(source);
            assert_eq!(bin_op.op, op, "{source}");
            assert_eq!(bin_op.right, node.value, "{source}");
            // The left operand is the target itself, moved to load position.
            let mut target = node.target.as_ref().clone();
            match &mut target {
                Expr::Name(name) => name.ctx = ExprContext::Load,
                Expr::Attribute(attr) => attr.ctx = ExprContext::Load,
                _ => {}
            }
            if let Expr::Subscript(subscript) = &mut target {
                subscript.ctx = ExprContext::Load;
            }
            assert_eq!(bin_op.left.as_ref(), &target, "{source}");
        }
    }
}
