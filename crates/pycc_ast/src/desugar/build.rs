//! Constructors for the synthesized nodes [`super`] splices into a module.
//! Every node takes the range of the source construct it replaces, so a
//! diagnostic on desugared code still points at what the user wrote.

use ruff_python_ast::name::Name;
use ruff_python_ast::{
    Arguments, AtomicNodeIndex, Comprehension, Expr, ExprAttribute, ExprCall, ExprContext,
    ExprListComp, ExprName, ExprNumberLiteral, ExprStringLiteral, ExprSubscript, ExprTuple,
    Identifier, Int, Keyword, Number, Stmt, StmtAssign, StmtAugAssign, StmtFor, StringLiteral, StringLiteralFlags, StringLiteralValue,
};
use ruff_text_size::TextRange;

pub(super) fn name(id: &str, ctx: ExprContext, range: TextRange) -> Expr {
    Expr::Name(ExprName {
        node_index: AtomicNodeIndex::default(),
        range,
        id: Name::new(id),
        ctx,
    })
}

pub(super) fn load(id: &str, range: TextRange) -> Expr {
    name(id, ExprContext::Load, range)
}

pub(super) fn store(id: &str, range: TextRange) -> Expr {
    name(id, ExprContext::Store, range)
}

pub(super) fn int(value: u64, range: TextRange) -> Expr {
    Expr::NumberLiteral(ExprNumberLiteral {
        node_index: AtomicNodeIndex::default(),
        range,
        value: Number::Int(Int::from(value)),
    })
}

pub(super) fn string(value: &str, range: TextRange) -> Expr {
    Expr::StringLiteral(ExprStringLiteral {
        node_index: AtomicNodeIndex::default(),
        range,
        value: StringLiteralValue::single(StringLiteral {
            range,
            node_index: AtomicNodeIndex::default(),
            value: value.into(),
            flags: StringLiteralFlags::empty(),
        }),
    })
}

pub(super) fn call(func: Expr, args: Vec<Expr>, range: TextRange) -> Expr {
    call_kw(func, args, Vec::new(), range)
}

pub(super) fn call_kw(func: Expr, args: Vec<Expr>, keywords: Vec<Keyword>, range: TextRange) -> Expr {
    Expr::Call(ExprCall {
        node_index: AtomicNodeIndex::default(),
        range,
        func: Box::new(func),
        arguments: Arguments {
            range,
            node_index: AtomicNodeIndex::default(),
            args: args.into_boxed_slice(),
            keywords: keywords.into(),
        },
    })
}

pub(super) fn call_name(func: &str, args: Vec<Expr>, range: TextRange) -> Expr {
    call(load(func, range), args, range)
}

pub(super) fn attribute(value: Expr, attr: &str, range: TextRange) -> Expr {
    Expr::Attribute(ExprAttribute {
        node_index: AtomicNodeIndex::default(),
        range,
        value: Box::new(value),
        attr: Identifier::new(attr, range),
        ctx: ExprContext::Load,
    })
}

pub(super) fn subscript(value: Expr, index: Expr, ctx: ExprContext, range: TextRange) -> Expr {
    Expr::Subscript(ExprSubscript {
        node_index: AtomicNodeIndex::default(),
        range,
        value: Box::new(value),
        slice: Box::new(index),
        ctx,
    })
}

pub(super) fn tuple(elts: Vec<Expr>, ctx: ExprContext, range: TextRange) -> Expr {
    Expr::Tuple(ExprTuple {
        node_index: AtomicNodeIndex::default(),
        range,
        elts,
        ctx,
        parenthesized: true,
    })
}

pub(super) fn list_comp(elt: Expr, target: Expr, iter: Expr, ifs: Vec<Expr>, range: TextRange) -> Expr {
    Expr::ListComp(ExprListComp {
        node_index: AtomicNodeIndex::default(),
        range,
        elt: Box::new(elt),
        generators: vec![Comprehension {
            range,
            node_index: AtomicNodeIndex::default(),
            target,
            iter,
            ifs,
            is_async: false,
        }],
    })
}

pub(super) fn assign(target: Expr, value: Expr, range: TextRange) -> Stmt {
    Stmt::Assign(StmtAssign {
        node_index: AtomicNodeIndex::default(),
        range,
        targets: vec![target],
        value: Box::new(value),
    })
}

/// `target: annotation = value` for a plain name `target`.
pub(super) fn ann_assign(target: Expr, annotation: Expr, value: Expr, range: TextRange) -> Stmt {
    Stmt::AnnAssign(ruff_python_ast::StmtAnnAssign {
        node_index: AtomicNodeIndex::default(),
        range,
        target: Box::new(target),
        annotation: Box::new(annotation),
        value: Some(Box::new(value)),
        simple: true,
    })
}

pub(super) fn aug_assign(target: Expr, op: ruff_python_ast::Operator, value: Expr, range: TextRange) -> Stmt {
    Stmt::AugAssign(StmtAugAssign {
        node_index: AtomicNodeIndex::default(),
        range,
        target: Box::new(target),
        op,
        value: Box::new(value),
    })
}



pub(super) fn for_loop(target: Expr, iter: Expr, body: Vec<Stmt>, orelse: Vec<Stmt>, range: TextRange) -> Stmt {
    Stmt::For(StmtFor {
        node_index: AtomicNodeIndex::default(),
        range,
        is_async: false,
        target: Box::new(target),
        iter: Box::new(iter),
        body: body.into(),
        orelse: orelse.into(),
    })
}
