//! The direct sub-expressions of a `HirExpr`, shared and mutable, for the
//! typed-array lowering's rewrite (`super::lower`).
//!
//! Both matches are exhaustive with no wildcard arm, so a new `HirExpr`
//! variant is a compile error here rather than a subtree the rewrite
//! silently skips.

use crate::{CompElt, CompIter, ContainerReceiver, FStringPart, HirExpr};

/// The direct sub-expressions of `expr`, in evaluation order.
#[must_use]
pub fn expr_children(expr: &HirExpr) -> Vec<&HirExpr> {
    let mut out: Vec<&HirExpr> = Vec::new();
    match expr {
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::OpaqueBool
        | HirExpr::StringLiteral(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptySet(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::Super => {}
        HirExpr::Call { args, .. }
        | HirExpr::GenericClassInstantiate { args, .. }
        | HirExpr::ReceiverClassCall { args }
        | HirExpr::ListLiteral(args)
        | HirExpr::ObjectList(args)
        | HirExpr::SetLiteral(args)
        | HirExpr::TupleLiteral(args) => out.extend(args.iter()),
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            out.push(left);
            out.push(right);
        }
        HirExpr::CompareChain { first, links } => {
            out.push(first);
            out.extend(links.iter().map(|link| &link.right));
        }
        HirExpr::UnaryOp { operand: inner, .. }
        | HirExpr::Unpack { value: inner, .. }
        | HirExpr::NamedExpr { value: inner, .. }
        | HirExpr::ReceiverDispatchedCall { call: inner, .. } => out.push(inner),
        HirExpr::IfExp { test, body, orelse } => out.extend([test.as_ref(), body, orelse]),
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(e) = part {
                    out.push(e);
                }
            }
        }
        HirExpr::Subscript { base, index } => out.extend([base.as_ref(), index]),
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            out.push(base);
            out.extend([start, stop, step].into_iter().flatten().map(AsRef::as_ref));
        }
        HirExpr::ListAppend { list, value } => {
            if let ContainerReceiver::Attr(receiver) = list {
                out.push(receiver);
            }
            out.push(value);
        }
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                out.push(k);
                out.push(v);
            }
        }
        HirExpr::ListPop { list } => {
            if let ContainerReceiver::Attr(receiver) = list {
                out.push(receiver);
            }
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            if let ContainerReceiver::Attr(receiver) = dict {
                out.push(receiver);
            }
            out.push(key);
            out.push(default);
        }
        HirExpr::SetAdd { value, .. } => out.push(value),
        HirExpr::AttrGet { base, .. } => out.push(base),
        HirExpr::MethodCall { base, args, .. } => {
            out.push(base);
            out.extend(args.iter());
        }
        HirExpr::ExprCall { callee, args } => {
            out.push(callee);
            out.extend(args.iter());
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            out.push(call);
            out.extend(keywords.iter().map(|(_, v)| v));
        }
        HirExpr::Comprehension(comp) => out.extend(comp.sub_exprs()),
        HirExpr::Nd(nd) => {
            out.push(&nd.base);
            out.extend(nd.args.iter());
        }
    }
    out
}

/// [`expr_children`], mutable.
pub fn expr_children_mut(expr: &mut HirExpr) -> Vec<&mut HirExpr> {
    let mut out: Vec<&mut HirExpr> = Vec::new();
    match expr {
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::OpaqueBool
        | HirExpr::StringLiteral(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptySet(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::Super => {}
        HirExpr::Call { args, .. }
        | HirExpr::GenericClassInstantiate { args, .. }
        | HirExpr::ReceiverClassCall { args }
        | HirExpr::ListLiteral(args)
        | HirExpr::ObjectList(args)
        | HirExpr::SetLiteral(args)
        | HirExpr::TupleLiteral(args) => out.extend(args.iter_mut()),
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            out.push(left);
            out.push(right);
        }
        HirExpr::CompareChain { first, links } => {
            out.push(first);
            out.extend(links.iter_mut().map(|link| &mut link.right));
        }
        HirExpr::UnaryOp { operand: inner, .. }
        | HirExpr::Unpack { value: inner, .. }
        | HirExpr::NamedExpr { value: inner, .. }
        | HirExpr::ReceiverDispatchedCall { call: inner, .. } => out.push(inner),
        HirExpr::IfExp { test, body, orelse } => {
            out.push(test);
            out.push(body);
            out.push(orelse);
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(e) = part {
                    out.push(e);
                }
            }
        }
        HirExpr::Subscript { base, index } => {
            out.push(base);
            out.push(index);
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            out.push(base);
            out.extend([start, stop, step].into_iter().flatten().map(AsMut::as_mut));
        }
        HirExpr::ListAppend { list, value } => {
            if let ContainerReceiver::Attr(receiver) = list {
                out.push(receiver);
            }
            out.push(value);
        }
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                out.push(k);
                out.push(v);
            }
        }
        HirExpr::ListPop { list } => {
            if let ContainerReceiver::Attr(receiver) = list {
                out.push(receiver);
            }
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            if let ContainerReceiver::Attr(receiver) = dict {
                out.push(receiver);
            }
            out.push(key);
            out.push(default);
        }
        HirExpr::SetAdd { value, .. } => out.push(value),
        HirExpr::AttrGet { base, .. } => out.push(base),
        HirExpr::MethodCall { base, args, .. } => {
            out.push(base);
            out.extend(args.iter_mut());
        }
        HirExpr::ExprCall { callee, args } => {
            out.push(callee);
            out.extend(args.iter_mut());
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            out.push(call);
            out.extend(keywords.iter_mut().map(|(_, v)| v));
        }
        HirExpr::Comprehension(comp) => {
            match &mut comp.iter {
                CompIter::Range { start, stop, step } => out.extend([start, stop, step]),
                CompIter::Iterable(iterable) => out.push(iterable),
                CompIter::Name(_) => {}
            }
            if let Some(cond) = &mut comp.cond {
                out.push(cond);
            }
            match &mut comp.elt {
                CompElt::List(e) | CompElt::Set(e) => out.push(e),
                CompElt::Dict { key, value } => {
                    out.push(key);
                    out.push(value);
                }
            }
        }
        HirExpr::Nd(nd) => {
            out.push(&mut nd.base);
            out.extend(nd.args.iter_mut());
        }
    }
    out
}
