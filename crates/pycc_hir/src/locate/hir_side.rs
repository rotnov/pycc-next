//! The HIR half of [`super`]: node keys and walk order, mirroring
//! `ast_side`.

use super::NodeKey;
use crate::{
    BinOpKind, CmpOpKind, CompElt, CompIter, ContainerReceiver, FStringPart, HirExpr, HirStmt,
    UnaryOpKind,
};

/// Every statement of `body`, nested ones included, in pre-order.
pub(super) fn collect_stmts<'a>(body: &'a [HirStmt], out: &mut Vec<&'a HirStmt>) {
    for stmt in body {
        collect_stmt(stmt, out);
    }
}

fn collect_stmt<'a>(stmt: &'a HirStmt, out: &mut Vec<&'a HirStmt>) {
    out.push(stmt);
    match stmt {
        HirStmt::If { body, orelse, .. } => {
            collect_stmts(body, out);
            collect_stmts(orelse, out);
        }
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => collect_stmts(body, out),
        HirStmt::LoopElse { loop_stmt, orelse } => {
            collect_stmt(loop_stmt, out);
            collect_stmts(orelse, out);
        }
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            collect_stmts(body, out);
            for handler in handlers {
                collect_stmts(&handler.body, out);
            }
            collect_stmts(orelse, out);
            collect_stmts(finalbody, out);
        }
        HirStmt::Match { cases, .. } => {
            for case in cases {
                collect_stmts(&case.body, out);
            }
        }
        HirStmt::ExprStmt(_)
        | HirStmt::Assign { .. }
        | HirStmt::AnnAssign { .. }
        | HirStmt::DictSet { .. }
        | HirStmt::ListCompAssign { .. }
        | HirStmt::DictCompAssign { .. }
        | HirStmt::SetCompAssign { .. }
        | HirStmt::Return(_)
        | HirStmt::AttrSet { .. }
        | HirStmt::Raise { .. }
        | HirStmt::Delete { .. }
        | HirStmt::DeleteSlice { .. }
        | HirStmt::SetSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::DeleteItem { .. }
        | HirStmt::ForeignImport { .. }
        | HirStmt::Break
        | HirStmt::Continue => {}
    }
}

pub(super) fn stmt_key(stmt: &HirStmt) -> Option<NodeKey> {
    let key = match stmt {
        HirStmt::ExprStmt(_) => "expr".to_string(),
        HirStmt::Assign { target, .. }
        | HirStmt::AnnAssign { target, .. }
        | HirStmt::ListCompAssign { target, .. }
        | HirStmt::DictCompAssign { target, .. }
        | HirStmt::SetCompAssign { target, .. } => format!("={target}"),
        HirStmt::If { .. } => "if".into(),
        HirStmt::While { .. } => "while".into(),
        HirStmt::ForRange { var, .. }
        | HirStmt::ForList { var, .. }
        | HirStmt::ForObject { var, .. } => format!("for:{var}"),
        HirStmt::DictSet { .. } => "[]=".into(),
        HirStmt::Return(_) => "return".into(),
        HirStmt::AttrSet { attr, .. } => format!("attr=:{attr}"),
        HirStmt::Match { .. } => "match".into(),
        HirStmt::Try { .. } | HirStmt::TryStar { .. } => "try".into(),
        HirStmt::Raise { .. } => "raise".into(),
        HirStmt::Delete { name } => format!("del:{name}"),
        HirStmt::DeleteSlice { .. } | HirStmt::DeleteAttr { .. } | HirStmt::DeleteItem { .. } => {
            "del".into()
        }
        HirStmt::SetSlice { .. } => "[:]=".into(),
        HirStmt::ForeignImport { .. } => "import".into(),
        HirStmt::Break => "break".into(),
        HirStmt::Continue => "continue".into(),
        HirStmt::LoopElse { .. } => return None,
    };
    Some(NodeKey::new(key))
}

/// The expressions `stmt` owns (not those of its nested statements), in
/// source order.
pub(super) fn stmt_own_exprs<'a>(stmt: &'a HirStmt, visit: &mut dyn FnMut(&'a HirExpr)) {
    match stmt {
        HirStmt::ExprStmt(value) | HirStmt::Assign { value, .. } => visit(value),
        HirStmt::AnnAssign { value, .. } | HirStmt::Return(value) => {
            if let Some(value) = value {
                visit(value);
            }
        }
        HirStmt::If { test, .. } | HirStmt::While { test, .. } => visit(test),
        HirStmt::ForRange {
            start, stop, step, ..
        } => {
            visit(start);
            visit(stop);
            visit(step);
        }
        HirStmt::ForObject { iter, .. } => visit(iter),
        HirStmt::DictSet { key, value, .. } => {
            visit(key);
            visit(value);
        }
        HirStmt::ListCompAssign {
            iter, cond, elt, ..
        }
        | HirStmt::SetCompAssign {
            iter, cond, elt, ..
        } => {
            comp_iter(iter, visit);
            if let Some(cond) = cond {
                visit(cond);
            }
            visit(elt);
        }
        HirStmt::DictCompAssign {
            iter,
            cond,
            key,
            value,
            ..
        } => {
            comp_iter(iter, visit);
            if let Some(cond) = cond {
                visit(cond);
            }
            visit(key);
            visit(value);
        }
        HirStmt::AttrSet { base, value, .. } => {
            visit(base);
            visit(value);
        }
        HirStmt::Match { subject, cases } => {
            visit(subject);
            for case in cases {
                if let Some(guard) = &case.guard {
                    visit(guard);
                }
            }
        }
        HirStmt::Raise { exc, cause } => {
            for expr in [exc, cause].into_iter().flatten() {
                visit(expr);
            }
        }
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            ..
        } => {
            visit(base);
            for bound in [start, stop, step].into_iter().flatten() {
                visit(bound);
            }
        }
        HirStmt::SetSlice {
            base,
            start,
            stop,
            step,
            value,
            ..
        } => {
            visit(value);
            visit(base);
            for bound in [start, stop, step].into_iter().flatten() {
                visit(bound);
            }
        }
        HirStmt::DeleteAttr { base, .. } => visit(base),
        HirStmt::DeleteItem { base, key, .. } => {
            visit(base);
            visit(key);
        }
        HirStmt::ForList { .. }
        | HirStmt::LoopElse { .. }
        | HirStmt::Try { .. }
        | HirStmt::TryStar { .. }
        | HirStmt::Delete { .. }
        | HirStmt::ForeignImport { .. }
        | HirStmt::Break
        | HirStmt::Continue => {}
    }
}

fn comp_iter<'a>(iter: &'a CompIter, visit: &mut dyn FnMut(&'a HirExpr)) {
    match iter {
        CompIter::Range { start, stop, step } => {
            visit(start);
            visit(stop);
            visit(step);
        }
        CompIter::Iterable(iterable) => visit(iterable),
        CompIter::Name(_) => {}
    }
}

/// `expr` and every expression nested in it, in pre-order.
pub(super) fn collect_exprs<'a>(expr: &'a HirExpr, out: &mut Vec<&'a HirExpr>) {
    out.push(expr);
    let mut child = |expr: &'a HirExpr| collect_exprs(expr, out);
    let receiver = |receiver: &'a ContainerReceiver| receiver.attr_expr();
    match expr {
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::OpaqueBool
        | HirExpr::StringLiteral(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Ellipsis
        | HirExpr::Name(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptySet(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::Super => {}
        HirExpr::Nd(nd) => {
            child(&nd.base);
            nd.args.iter().for_each(child);
        }
        HirExpr::Call { args, .. }
        | HirExpr::ListLiteral(args)
        | HirExpr::ObjectList(args)
        | HirExpr::SetLiteral(args)
        | HirExpr::TupleLiteral(args)
        | HirExpr::GenericClassInstantiate { args, .. }
        | HirExpr::ReceiverClassCall { args } => args.iter().for_each(child),
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            child(left);
            child(right);
        }
        HirExpr::CompareChain { first, links } => {
            child(first);
            for link in links {
                child(&link.right);
            }
        }
        HirExpr::UnaryOp { operand, .. } => child(operand),
        HirExpr::IfExp { test, body, orelse } => {
            child(test);
            child(body);
            child(orelse);
        }
        HirExpr::Unpack { value, .. } | HirExpr::NamedExpr { value, .. } => child(value),
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(expr) = part {
                    child(expr);
                }
            }
        }
        HirExpr::Subscript { base, index } => {
            child(base);
            child(index);
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            child(base);
            for bound in [start, stop, step].into_iter().flatten() {
                child(bound);
            }
        }
        HirExpr::ListAppend { list, value } => {
            receiver(list).into_iter().for_each(&mut child);
            child(value);
        }
        HirExpr::DictLiteral(pairs) => {
            for (key, value) in pairs {
                child(key);
                child(value);
            }
        }
        HirExpr::ListPop { list } => receiver(list).into_iter().for_each(child),
        HirExpr::DictGetOrDefault { dict, key, default } => {
            receiver(dict).into_iter().for_each(&mut child);
            child(key);
            child(default);
        }
        HirExpr::SetAdd { value, .. } => child(value),
        HirExpr::AttrGet { base, .. } => child(base),
        HirExpr::MethodCall { base, args, .. } => {
            child(base);
            args.iter().for_each(child);
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => child(call),
        HirExpr::ExprCall { callee, args } => {
            child(callee);
            args.iter().for_each(child);
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            child(call);
            for (_, value) in keywords {
                child(value);
            }
        }
        HirExpr::Comprehension(comp) => {
            comp_iter(&comp.iter, &mut child);
            if let Some(cond) = &comp.cond {
                child(cond);
            }
            match &comp.elt {
                CompElt::List(elt) | CompElt::Set(elt) => child(elt),
                CompElt::Dict { key, value } => {
                    child(key);
                    child(value);
                }
            }
        }
    }
}

pub(super) fn expr_key(expr: &HirExpr) -> Option<NodeKey> {
    let key = match expr {
        HirExpr::IntLiteral(value) => format!("i:{value}"),
        HirExpr::FloatLiteral(value) => format!("f:{}", value.to_bits()),
        HirExpr::BoolLiteral(value) => format!("b:{}", if *value { "True" } else { "False" }),
        HirExpr::StringLiteral(value) => format!("s:{value}"),
        HirExpr::NoneLiteral => "none".into(),
        HirExpr::NotImplemented => "n:NotImplemented".into(),
        HirExpr::Ellipsis => "n:Ellipsis".into(),
        HirExpr::Name(name) => format!("n:{name}"),
        HirExpr::Call { callee, .. } | HirExpr::GenericClassInstantiate { class: callee, .. } => {
            format!("c:{callee}")
        }
        HirExpr::Super => "c:super".into(),
        HirExpr::BinOp { op, .. } | HirExpr::AugBinOp { op, .. } => {
            format!("bin:{}", bin_op(*op))
        }
        HirExpr::Compare { op, .. } => format!("cmp:{}", cmp_op(*op)),
        HirExpr::CompareChain { .. } => "cmpchain".into(),
        HirExpr::UnaryOp { op, .. } => format!("un:{}", unary_op(*op)),
        HirExpr::BoolOp { op, .. } => format!("bool:{}", op.as_str()),
        HirExpr::IfExp { .. } => "ifexp".into(),
        HirExpr::FString(_) => "fstr".into(),
        HirExpr::ListLiteral(_) | HirExpr::EmptyList(_) | HirExpr::ObjectList(_) => "list".into(),
        HirExpr::DictLiteral(_) | HirExpr::EmptyDict(_) => "dict".into(),
        HirExpr::SetLiteral(_) | HirExpr::EmptySet(_) => "set".into(),
        HirExpr::TupleLiteral(_) => "tuple".into(),
        HirExpr::Comprehension(_) => "comp".into(),
        HirExpr::Subscript { .. } => "[]".into(),
        HirExpr::Slice { .. } => "[:]".into(),
        HirExpr::AttrGet { attr, .. } => format!("a:{attr}"),
        HirExpr::MethodCall { method, .. } => format!("m:{method}"),
        HirExpr::ListAppend { .. } => "m:append".into(),
        HirExpr::ListPop { .. } => "m:pop".into(),
        HirExpr::DictGetOrDefault { .. } => "m:get".into(),
        HirExpr::SetAdd { .. } => "m:add".into(),
        HirExpr::ExprCall { .. } => "call()".into(),
        HirExpr::NamedExpr { name, .. } => format!("walrus:{name}"),
        HirExpr::OpaqueBool
        | HirExpr::Unpack { .. }
        | HirExpr::ReceiverDispatchedCall { .. }
        | HirExpr::KeywordCall { .. }
        | HirExpr::ReceiverClassCall { .. } => return None,
        HirExpr::Nd(nd) => match nd.op {
            crate::ndarray::NdOp::Get { .. } => "[]".into(),
            crate::ndarray::NdOp::Shape => "a:shape".into(),
            crate::ndarray::NdOp::Size => "a:size".into(),
            crate::ndarray::NdOp::Ndim => "a:ndim".into(),
            crate::ndarray::NdOp::Set { .. }
            | crate::ndarray::NdOp::Dim(_)
            | crate::ndarray::NdOp::Check => {
                return None;
            }
        },
    };
    Some(NodeKey::new(key))
}

fn bin_op(op: BinOpKind) -> &'static str {
    op.as_str()
}

fn cmp_op(op: CmpOpKind) -> &'static str {
    match op {
        CmpOpKind::Eq => "==",
        CmpOpKind::NotEq => "!=",
        CmpOpKind::Lt => "<",
        CmpOpKind::LtE => "<=",
        CmpOpKind::Gt => ">",
        CmpOpKind::GtE => ">=",
        CmpOpKind::Is => "is",
        CmpOpKind::IsNot => "is not",
        CmpOpKind::In => "in",
        CmpOpKind::NotIn => "not in",
    }
}

fn unary_op(op: UnaryOpKind) -> &'static str {
    match op {
        UnaryOpKind::USub => "-",
        UnaryOpKind::UAdd => "+",
        UnaryOpKind::Not => "not",
        UnaryOpKind::Invert => "~",
    }
}
