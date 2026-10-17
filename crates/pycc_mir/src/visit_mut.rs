//! The mutable twin of `visit.rs`: the direct sub-expressions of an
//! expression and the expressions of a statement, in the same order, for a
//! pass that rewrites them in place.
//!
//! Both matches are exhaustive with no wildcard arm, so a new variant is a
//! compile error here, as it is in `visit.rs`.

use crate::{
    CompSource, InstanceOrderExpr, MirCompElt, MirContainerReceiver, MirExceptionValue, MirExpr,
    MirFStringPart, MirStmt, ObjIsInstanceClass,
};

impl MirExpr {
    /// [`MirExpr::children`], mutably.
    pub fn children_mut(&mut self) -> Vec<&mut MirExpr> {
        let mut out: Vec<&mut MirExpr> = Vec::new();
        match self {
            MirExpr::IntLiteral(_)
            | MirExpr::FloatLiteral(_)
            | MirExpr::BoolLiteral(_)
            | MirExpr::StringLiteral(_)
            | MirExpr::EmptyList(_)
            | MirExpr::EmptyDict(_)
            | MirExpr::EmptySet(_)
            | MirExpr::NoneLiteral
            | MirExpr::NotImplemented
            | MirExpr::Name { .. }
            | MirExpr::NullInstance { .. } => {}
            MirExpr::IntBoundary(inner)
            | MirExpr::OptionalWrap(inner, _)
            | MirExpr::NumWrap(inner)
            | MirExpr::ObjectBox(inner)
            | MirExpr::OptionalUnwrap(inner, _)
            | MirExpr::ObjectUnbox(inner, _)
            | MirExpr::ObjectCoerce(inner, _)
            | MirExpr::TupleRebox(inner, _)
            | MirExpr::ObjectIndex(inner, _)
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
            | MirExpr::NamedExpr { value: inner, .. }
            | MirExpr::GenToList {
                generator: inner, ..
            }
            | MirExpr::GenAnyAll {
                generator: inner, ..
            }
            | MirExpr::GenClose { generator: inner }
            | MirExpr::GenYield { value: inner, .. }
            | MirExpr::GenYieldFrom { sub: inner, .. }
            | MirExpr::StopIterationValue {
                exception: inner, ..
            } => out.push(inner),
            MirExpr::GenSend {
                generator, value, ..
            } => {
                out.push(generator);
                out.push(value);
            }
            MirExpr::GenThrow {
                generator,
                exception,
                ..
            } => {
                out.push(generator);
                out.extend(exception_value_exprs_mut(exception));
            }
            MirExpr::GenNext {
                generator, default, ..
            } => {
                out.push(generator);
                out.extend(default.iter_mut().map(|d| &mut **d));
            }
            MirExpr::Call { args, .. } => out.extend(args),
            MirExpr::MakeClosure { captures, .. } => out.extend(captures),
            MirExpr::CallClosure { callee, args, .. } => {
                out.push(callee);
                out.extend(args);
            }
            MirExpr::RtCall(call) => out.extend(&mut call.args),
            MirExpr::Nd(nd) => {
                out.push(&mut nd.base);
                out.extend(&mut nd.args);
            }
            MirExpr::Instantiate(inst) => out.extend(&mut inst.args),
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
                out.extend(links.iter_mut().map(|link| &mut link.right));
            }
            MirExpr::IfExp {
                test, body, orelse, ..
            } => out.extend([test.as_mut(), body.as_mut(), orelse.as_mut()]),
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
                out.extend(receiver_mut(list));
                out.push(value);
            }
            MirExpr::ListPop { list, index, .. } => {
                out.extend(receiver_mut(list));
                out.extend(index.as_deref_mut());
            }
            MirExpr::ListInsert { list, index, value } => {
                out.extend(receiver_mut(list));
                out.push(index);
                out.push(value);
            }
            MirExpr::DictGetOrDefault {
                dict, key, default, ..
            } => {
                out.extend(receiver_mut(dict));
                out.push(key);
                out.push(default);
            }
            MirExpr::SetAdd { value, .. } => out.push(value),
            MirExpr::InstanceOrder(order) => out.extend(order_operands_mut(order)),
            MirExpr::InstanceSetContains { element, set, .. } => {
                out.push(element);
                out.push(set);
            }
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
                out.extend(
                    [start, stop, step]
                        .into_iter()
                        .flatten()
                        .map(|b| &mut **b),
                );
            }
            MirExpr::FrozenSetFrom { source } => out.extend(source.as_deref_mut()),
            MirExpr::ObjMethodCall { base, args, .. } | MirExpr::ObjCall { callee: base, args } => {
                out.push(base);
                out.extend(args);
            }
            MirExpr::ObjKeywordCall(call) => {
                out.push(&mut call.call);
                out.extend(&mut call.values);
            }
            MirExpr::ObjIsInstance { value, class } => {
                out.push(value);
                out.extend(class_objects_mut(class));
            }
            MirExpr::Comprehension(comp) => {
                out.extend(comp_source_exprs_mut(&mut comp.source));
                out.extend(comp.cond.as_mut());
                match &mut comp.elt {
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

fn receiver_mut(receiver: &mut MirContainerReceiver) -> Option<&mut MirExpr> {
    match receiver {
        MirContainerReceiver::Name(_) => None,
        MirContainerReceiver::Attr(expr) => Some(expr),
    }
}

fn order_operands_mut(order: &mut InstanceOrderExpr) -> Vec<&mut MirExpr> {
    let mut out: Vec<&mut MirExpr> = order.target.iter_mut().collect();
    out.push(&mut order.list);
    out.extend(order.reverse.iter_mut());
    out
}

fn class_objects_mut(class: &mut ObjIsInstanceClass) -> Vec<&mut MirExpr> {
    match class {
        ObjIsInstanceClass::Object(class) => vec![class],
        ObjIsInstanceClass::AnyOf(classes) => classes.iter_mut().flat_map(class_objects_mut).collect(),
        ObjIsInstanceClass::Builtin(_) | ObjIsInstanceClass::Compiled(_) => Vec::new(),
    }
}

fn comp_source_exprs_mut(source: &mut CompSource) -> Vec<&mut MirExpr> {
    match source {
        CompSource::Range { start, stop, step } => vec![start, stop, step],
        CompSource::Object(iter) | CompSource::ListExpr(iter) => vec![iter],
        CompSource::List(_) | CompSource::Dict(_) | CompSource::Set(_) => Vec::new(),
    }
}

fn exception_value_exprs_mut(value: &mut MirExceptionValue) -> Vec<&mut MirExpr> {
    match value {
        MirExceptionValue::Constructed { message, .. } => vec![message],
        MirExceptionValue::Existing(expr) => vec![expr],
        MirExceptionValue::ConstructedGroup {
            message, members, ..
        } => std::iter::once(message).chain(members).collect(),
    }
}

impl MirStmt {
    /// [`MirStmt::exprs`], mutably.
    pub fn exprs_mut(&mut self) -> Vec<&mut MirExpr> {
        match self {
            MirStmt::ExprStmt(expr)
            | MirStmt::Assign { value: expr, .. }
            | MirStmt::If { test: expr, .. }
            | MirStmt::While { test: expr, .. }
            | MirStmt::ForObject { iter: expr, .. }
            | MirStmt::Yield(expr)
            | MirStmt::GenReturn(expr)
            | MirStmt::ObjRaise {
                value: expr,
                cause: None,
            } => vec![expr],
            MirStmt::ObjRaise {
                value,
                cause: Some(cause),
            } => std::iter::once(value)
                .chain(exception_value_exprs_mut(cause))
                .collect(),
            MirStmt::Return(value) => value.iter_mut().collect(),
            MirStmt::ReturnBufferSlice { start, stop, .. } => {
                start.iter_mut().chain(stop.iter_mut()).collect()
            }
            MirStmt::AssignMulti(multi) => multi.args.iter_mut().collect(),
            MirStmt::SetOut { value, .. } => vec![value],
            MirStmt::ForRange {
                start, stop, step, ..
            } => vec![start, stop, step],
            MirStmt::DictSet { key, value, .. } => vec![key, value],
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
                .chain(start.iter_mut())
                .chain(stop.iter_mut())
                .chain(step.iter_mut())
                .collect(),
            MirStmt::ObjSetSlice {
                base,
                start,
                stop,
                step,
                value,
            } => [value, base]
                .into_iter()
                .chain(start.iter_mut())
                .chain(stop.iter_mut())
                .chain(step.iter_mut())
                .collect(),
            MirStmt::ListCompAssign {
                source, cond, elt, ..
            }
            | MirStmt::SetCompAssign {
                source, cond, elt, ..
            } => comp_source_exprs_mut(source)
                .into_iter()
                .chain(cond.as_deref_mut())
                .chain(std::iter::once(&mut **elt))
                .collect(),
            MirStmt::DictCompAssign {
                source,
                cond,
                key,
                value,
                ..
            } => comp_source_exprs_mut(source)
                .into_iter()
                .chain(cond.as_deref_mut())
                .chain([&mut **key, &mut **value])
                .collect(),
            MirStmt::Raise { exception, .. } => exception_value_exprs_mut(exception),
            MirStmt::RaiseFrom {
                exception, cause, ..
            } => exception_value_exprs_mut(exception)
                .into_iter()
                .chain(exception_value_exprs_mut(cause))
                .collect(),
            MirStmt::NoOp
            | MirStmt::ReleaseName(_)
            | MirStmt::Unreachable
            | MirStmt::ForList { .. }
            | MirStmt::ForGen { .. }
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
}
