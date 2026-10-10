//! A structural walk over MIR: the direct sub-expressions of an expression
//! and the expressions and nested bodies of a statement, in the order code
//! generation evaluates them.
//!
//! Both matches are exhaustive with no wildcard arm, so a new variant is a
//! compile error here rather than a node an analysis silently skips.

use crate::{
    CompSource, MirCompElt, MirExceptionValue, MirExpr, MirFStringPart, MirStmt, ObjIsInstanceClass,
};

impl MirExpr {
    /// The direct sub-expressions of `self`, in evaluation order.
    ///
    /// A container receiver read through an attribute
    /// (`MirContainerReceiver::Attr`) is a child; a bare-name receiver
    /// (`ListAppend`'s `Name`, `SetAdd`'s `set`) is not an expression and
    /// is not listed. A comprehension lists its source, condition and
    /// element.
    #[must_use]
    pub fn children(&self) -> Vec<&MirExpr> {
        let mut out: Vec<&MirExpr> = Vec::new();
        match self {
            MirExpr::IntLiteral(_)
            | MirExpr::FloatLiteral(_)
            | MirExpr::BoolLiteral(_)
            | MirExpr::StringLiteral(_)
            | MirExpr::EmptyList(_)
            | MirExpr::EmptyDict(_)
            | MirExpr::NoneLiteral
            | MirExpr::NotImplemented
            | MirExpr::Name { .. }
            | MirExpr::NullInstance { .. } => {}
            MirExpr::IntBoundary(inner)
            | MirExpr::OptionalWrap(inner, _)
            | MirExpr::ObjectBox(inner)
            | MirExpr::OptionalUnwrap(inner, _)
            | MirExpr::ObjectUnbox(inner, _)
            | MirExpr::Not(inner)
            | MirExpr::ExceptionMessage(inner)
            | MirExpr::ExceptionTypeTest { obj: inner, .. }
            | MirExpr::InstanceHash { operand: inner, .. }
            | MirExpr::ObjUnaryOp { operand: inner, .. }
            | MirExpr::AttrGet { base: inner, .. }
            | MirExpr::ObjAttrGet { base: inner, .. }
            | MirExpr::ObjLen { base: inner }
            | MirExpr::ObjType { base: inner }
            | MirExpr::BufferLen { base: inner }
            | MirExpr::BufferAlloc { len: inner }
            | MirExpr::ObjUnpackFloatTuple { base: inner, .. }
            | MirExpr::ObjUnpack { value: inner, .. }
            | MirExpr::NamedExpr { value: inner, .. } => out.push(inner),
            MirExpr::Call { args, .. } => out.extend(args),
            MirExpr::RtCall(call) => out.extend(&call.args),
            MirExpr::Instantiate(inst) => out.extend(&inst.args),
            MirExpr::BinOp { left, right, .. }
            | MirExpr::Compare { left, right, .. }
            | MirExpr::BoolOp { left, right, .. }
            | MirExpr::ObjCompare { left, right, .. }
            | MirExpr::ObjBinOp { left, right, .. }
            | MirExpr::ObjContains {
                item: left,
                container: right,
                ..
            }
            | MirExpr::Subscript {
                base: left,
                index: right,
            }
            | MirExpr::DictGet {
                dict: left,
                key: right,
            }
            | MirExpr::ObjSubscript {
                base: left,
                index: right,
            }
            | MirExpr::BufferGet {
                base: left,
                index: right,
            }
            | MirExpr::Sequence {
                discard: left,
                value: right,
            } => {
                out.push(left);
                out.push(right);
            }
            MirExpr::CompareChain { first, links } => {
                out.push(first);
                out.extend(links.iter().map(|link| &link.right));
            }
            MirExpr::IfExp {
                test, body, orelse, ..
            } => out.extend([test.as_ref(), body.as_ref(), orelse.as_ref()]),
            MirExpr::FString(parts) => {
                for part in parts {
                    if let MirFStringPart::Interpolation(inner) = part {
                        out.push(inner);
                    }
                }
            }
            MirExpr::ListLiteral(elements)
            | MirExpr::ObjList { elements }
            | MirExpr::ObjDisplay { elements, .. }
            | MirExpr::SetLiteral { elements, .. }
            | MirExpr::TupleLiteral(elements) => out.extend(elements),
            MirExpr::ListAppend { list, value } => {
                out.extend(list.attr_expr());
                out.push(value);
            }
            MirExpr::ListPop { list, index, .. } => {
                out.extend(list.attr_expr());
                out.extend(index.as_deref());
            }
            MirExpr::ListInsert { list, index, value } => {
                out.extend(list.attr_expr());
                out.push(index);
                out.push(value);
            }
            MirExpr::DictGetOrDefault {
                dict, key, default, ..
            } => {
                out.extend(dict.attr_expr());
                out.push(key);
                out.push(default);
            }
            MirExpr::SetAdd { value, .. } => out.push(value),
            MirExpr::DictLiteral(pairs) => {
                for (key, value) in pairs {
                    out.push(key);
                    out.push(value);
                }
            }
            MirExpr::Slice {
                base,
                start,
                stop,
                step,
            }
            | MirExpr::ObjSlice {
                base,
                start,
                stop,
                step,
            } => {
                out.push(base);
                out.extend([start, stop, step].into_iter().flatten().map(|b| &**b));
            }
            MirExpr::FrozenSetFrom { source } => out.extend(source.as_deref()),
            MirExpr::ObjMethodCall { base, args, .. } | MirExpr::ObjCall { callee: base, args } => {
                out.push(base);
                out.extend(args);
            }
            MirExpr::ObjKeywordCall(call) => {
                out.push(&call.call);
                out.extend(&call.values);
            }
            MirExpr::ObjIsInstance { value, class } => {
                out.push(value);
                if let ObjIsInstanceClass::Object(class) = class {
                    out.push(class);
                }
            }
            MirExpr::Comprehension(comp) => {
                out.extend(comp_source_exprs(&comp.source));
                out.extend(comp.cond.as_ref());
                match &comp.elt {
                    MirCompElt::List(elt) | MirCompElt::Set(elt, _) => out.push(elt),
                    MirCompElt::Dict { key, value } => {
                        out.push(key);
                        out.push(value);
                    }
                }
            }
        }
        out
    }
}

/// The expressions of a comprehension source (`range` bounds, an object
/// or list-valued iterable); a named list, dict or set has none.
#[must_use]
pub fn comp_source_exprs(source: &CompSource) -> Vec<&MirExpr> {
    match source {
        CompSource::Range { start, stop, step } => vec![start, stop, step],
        CompSource::Object(iter) | CompSource::ListExpr(iter) => vec![iter],
        CompSource::List(_) | CompSource::Dict(_) | CompSource::Set(_) => Vec::new(),
    }
}

fn exception_value_exprs(value: &MirExceptionValue) -> Vec<&MirExpr> {
    match value {
        MirExceptionValue::Constructed { message, .. } => vec![message],
        MirExceptionValue::Existing(expr) => vec![expr],
        MirExceptionValue::ConstructedGroup {
            message, members, ..
        } => std::iter::once(message).chain(members).collect(),
    }
}

impl MirStmt {
    /// The expressions `self` evaluates itself, in evaluation order --
    /// not those of its nested bodies ([`MirStmt::bodies`]).
    #[must_use]
    pub fn exprs(&self) -> Vec<&MirExpr> {
        match self {
            MirStmt::ExprStmt(expr)
            | MirStmt::Assign { value: expr, .. }
            | MirStmt::If { test: expr, .. }
            | MirStmt::While { test: expr, .. }
            | MirStmt::ForObject { iter: expr, .. }
            | MirStmt::ObjRaise { value: expr } => vec![expr],
            MirStmt::Return(value) => value.iter().collect(),
            MirStmt::ReturnBufferSlice { start, stop, .. } => {
                start.iter().chain(stop.iter()).collect()
            }
            MirStmt::ForRange {
                start, stop, step, ..
            } => vec![start, stop, step],
            MirStmt::DictSet { key, value, .. } => vec![key, value],
            // Value first, then the target: CPython's order, and codegen's.
            MirStmt::BufferSet { base, index, value } | MirStmt::ListSet { base, index, value } => {
                vec![value, base, index]
            }
            MirStmt::AttrSet { base, value, .. } => vec![value, base],
            MirStmt::ObjAttrSet { base, value, .. } => vec![base, value],
            MirStmt::ObjSetItem { base, key, value } => vec![base, key, value],
            MirStmt::ObjDelItem { base, key } => vec![base, key],
            MirStmt::ObjDelAttr { base, .. } => vec![base],
            MirStmt::ObjDelSlice {
                base,
                start,
                stop,
                step,
            } => std::iter::once(base)
                .chain(start.iter())
                .chain(stop.iter())
                .chain(step.iter())
                .collect(),
            MirStmt::ListCompAssign {
                source, cond, elt, ..
            }
            | MirStmt::SetCompAssign {
                source, cond, elt, ..
            } => comp_source_exprs(source)
                .into_iter()
                .chain(cond.as_deref())
                .chain(std::iter::once(&**elt))
                .collect(),
            MirStmt::DictCompAssign {
                source,
                cond,
                key,
                value,
                ..
            } => comp_source_exprs(source)
                .into_iter()
                .chain(cond.as_deref())
                .chain([&**key, &**value])
                .collect(),
            MirStmt::Raise { exception, .. } => exception_value_exprs(exception),
            MirStmt::RaiseFrom {
                exception, cause, ..
            } => exception_value_exprs(exception)
                .into_iter()
                .chain(exception_value_exprs(cause))
                .collect(),
            MirStmt::NoOp
            | MirStmt::Unreachable
            | MirStmt::ForList { .. }
            | MirStmt::ForDict { .. }
            | MirStmt::ForSet { .. }
            | MirStmt::Seq(_)
            | MirStmt::Break
            | MirStmt::Continue
            | MirStmt::LoopElse { .. }
            | MirStmt::Try { .. }
            | MirStmt::TryStar { .. }
            | MirStmt::Reraise
            | MirStmt::ForeignImport { .. } => Vec::new(),
        }
    }

    /// The statement bodies nested in `self` (a loop's body, both arms of
    /// an `if`, every part of a `try`, a `Seq`'s statements, a loop-else's
    /// loop and `else`).
    #[must_use]
    pub fn bodies(&self) -> Vec<&[MirStmt]> {
        match self {
            MirStmt::If { body, orelse, .. } => vec![body, orelse],
            MirStmt::While { body, .. }
            | MirStmt::ForRange { body, .. }
            | MirStmt::ForList { body, .. }
            | MirStmt::ForObject { body, .. }
            | MirStmt::ForDict { body, .. }
            | MirStmt::ForSet { body, .. } => vec![body],
            MirStmt::Seq(stmts) => vec![stmts],
            MirStmt::LoopElse { loop_stmt, orelse } => {
                vec![std::slice::from_ref(&**loop_stmt), orelse]
            }
            MirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            }
            | MirStmt::TryStar {
                body,
                handlers,
                orelse,
                finalbody,
            } => std::iter::once(body.as_slice())
                .chain(handlers.iter().map(|h| h.body.as_slice()))
                .chain([orelse.as_slice(), finalbody.as_slice()])
                .collect(),
            MirStmt::ExprStmt(_)
            | MirStmt::Assign { .. }
            | MirStmt::NoOp
            | MirStmt::Unreachable
            | MirStmt::DictSet { .. }
            | MirStmt::BufferSet { .. }
            | MirStmt::ListSet { .. }
            | MirStmt::ListCompAssign { .. }
            | MirStmt::DictCompAssign { .. }
            | MirStmt::SetCompAssign { .. }
            | MirStmt::Return(_)
            | MirStmt::ReturnBufferSlice { .. }
            | MirStmt::AttrSet { .. }
            | MirStmt::ObjDelSlice { .. }
            | MirStmt::ObjAttrSet { .. }
            | MirStmt::ObjDelAttr { .. }
            | MirStmt::ObjSetItem { .. }
            | MirStmt::ObjDelItem { .. }
            | MirStmt::Break
            | MirStmt::Continue
            | MirStmt::Raise { .. }
            | MirStmt::RaiseFrom { .. }
            | MirStmt::Reraise
            | MirStmt::ObjRaise { .. }
            | MirStmt::ForeignImport { .. } => Vec::new(),
        }
    }
}
