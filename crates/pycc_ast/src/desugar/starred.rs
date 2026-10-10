//! `a, *rest = v` (and `first, *mid, last = v`): the value as a list, a
//! length check raising CPython's `ValueError`, then each name from an
//! index or, for the starred one, a slice (always a list, as in CPython).

use super::Fresh;
use super::build::{assign, call_name, int, load, store, string, subscript};
use ruff_python_ast::{
    AtomicNodeIndex, CmpOp, ElifElseClause, Expr, ExprBinOp, ExprCompare, ExprContext, ExprSlice,
    ExprUnaryOp, Operator, Stmt, StmtAssign, StmtIf, StmtRaise, UnaryOp,
};
use ruff_text_size::TextRange;

fn binop(left: Expr, op: Operator, right: Expr, range: TextRange) -> Expr {
    Expr::BinOp(ExprBinOp {
        node_index: AtomicNodeIndex::default(),
        range,
        left: Box::new(left),
        op,
        right: Box::new(right),
    })
}

fn negative(value: u64, range: TextRange) -> Expr {
    Expr::UnaryOp(ExprUnaryOp {
        node_index: AtomicNodeIndex::default(),
        range,
        op: UnaryOp::USub,
        operand: Box::new(int(value, range)),
    })
}

/// The names of a tuple/list target with exactly one starred name, every
/// element a plain name: `(before, starred, after)`.
fn split_target(target: &Expr) -> Option<(Vec<Expr>, Expr, Vec<Expr>)> {
    let elts = match target {
        Expr::Tuple(t) => &t.elts,
        Expr::List(l) => &l.elts,
        _ => return None,
    };
    let star = elts.iter().position(|e| matches!(e, Expr::Starred(_)))?;
    if elts.iter().filter(|e| matches!(e, Expr::Starred(_))).count() != 1 {
        return None;
    }
    let Expr::Starred(starred) = &elts[star] else {
        unreachable!()
    };
    let plain = |e: &Expr| matches!(e, Expr::Name(_));
    if !plain(&starred.value) || !elts.iter().enumerate().all(|(i, e)| i == star || plain(e)) {
        return None;
    }
    Some((
        elts[..star].to_vec(),
        (*starred.value).clone(),
        elts[star + 1..].to_vec(),
    ))
}

/// Expands `assign` when its one target is a starred unpacking.
pub(crate) fn expand_starred(assign_stmt: &StmtAssign, fresh: &Fresh) -> Option<Vec<Stmt>> {
    let [target] = assign_stmt.targets.as_slice() else {
        return None;
    };
    let (before, starred, after) = split_target(target)?;
    let range = assign_stmt.range;
    let tmp = fresh.name();
    let fixed = (before.len() + after.len()) as u64;
    let len = || call_name("len", vec![load(&tmp, range)], range);
    // A tuple or list display is its own list; anything else is iterated
    // into one.
    let as_list = match assign_stmt.value.as_ref() {
        Expr::Tuple(t) if !t.elts.iter().any(|e| matches!(e, Expr::Starred(_))) => {
            Expr::List(ruff_python_ast::ExprList {
                node_index: AtomicNodeIndex::default(),
                range: t.range,
                elts: t.elts.clone(),
                ctx: ExprContext::Load,
            })
        }
        value => call_name("list", vec![value.clone()], range),
    };
    let mut out = vec![assign(store(&tmp, range), as_list, range)];
    // CPython: `ValueError: not enough values to unpack (expected at least
    // N, got M)`.
    let message = binop(
        binop(
            string(
                &format!("not enough values to unpack (expected at least {fixed}, got "),
                range,
            ),
            Operator::Add,
            call_name("str", vec![len()], range),
            range,
        ),
        Operator::Add,
        string(")", range),
        range,
    );
    out.push(Stmt::If(StmtIf {
        node_index: AtomicNodeIndex::default(),
        range,
        test: Box::new(Expr::Compare(ExprCompare {
            node_index: AtomicNodeIndex::default(),
            range,
            left: Box::new(len()),
            ops: vec![CmpOp::Lt].into_boxed_slice(),
            comparators: vec![int(fixed, range)].into_boxed_slice(),
        })),
        body: vec![Stmt::Raise(StmtRaise {
            node_index: AtomicNodeIndex::default(),
            range,
            exc: Some(Box::new(call_name("ValueError", vec![message], range))),
            cause: None,
        })]
        .into(),
        elif_else_clauses: Vec::<ElifElseClause>::new(),
    }));
    for (i, name) in before.iter().enumerate() {
        out.push(assign(
            name.clone(),
            subscript(load(&tmp, range), int(i as u64, range), ExprContext::Load, range),
            range,
        ));
    }
    let upper = (!after.is_empty())
        .then(|| Box::new(binop(len(), Operator::Sub, int(after.len() as u64, range), range)));
    out.push(assign(
        starred,
        subscript(
            load(&tmp, range),
            Expr::Slice(ExprSlice {
                node_index: AtomicNodeIndex::default(),
                range,
                lower: Some(Box::new(int(before.len() as u64, range))),
                upper,
                step: None,
            }),
            ExprContext::Load,
            range,
        ),
        range,
    ));
    for (j, name) in after.iter().enumerate() {
        out.push(assign(
            name.clone(),
            subscript(
                load(&tmp, range),
                negative((after.len() - j) as u64, range),
                ExprContext::Load,
                range,
            ),
            range,
        ));
    }
    Some(out)
}
