//! The receiver-exact dispatch invariant, checked over a lowered module
//! (#1337, D-254 WI-5).
//!
//! **Invariant.** Every call to a user method runs the body the static
//! receiver class resolves the member to: the receiver's own item, the
//! receiver-exact copy of an inherited body (`pycc_hir::inherited_copy_name`)
//! when the module holds one, or else the inherited item itself. A
//! `super()` call instead runs the body `super()` selects for that receiver
//! from the calling body's anchor class.
//!
//! Every lowering site that picks a method body routes through
//! `receiver_exact::exact_callee`. This verifier is the mechanical guard
//! against a site that does not: it re-derives the expected callee from the
//! class tables and the lowered item set alone -- never through
//! `exact_callee`, so the check is not tautological -- and panics on a
//! mismatch. It runs under `cfg(debug_assertions)`, which every test build
//! has, so the whole test corpus exercises it.
//!
//! **Strength.** A lowered call does not record whether it came from
//! `self.x()` or `super().x()`: both are a `MirExpr::Call` with the
//! receiver as `args[0]`. Inside a method body the verifier therefore
//! accepts either the receiver-resolved body or the `super()` target from
//! the body's anchor, and rejects every other callee. A site that confused
//! the two routes inside one body would pass this check; that narrower
//! guarantee is deliberate (threading a call-origin marker through MIR for
//! a debug-only check is not worth the churn), and
//! `a_super_target_is_accepted_for_an_ordinary_call_too` in the tests pins
//! it.
//!
//! The walk is exhaustive over [`MirExpr`] and [`MirStmt`] with no
//! catch-all arm: a new variant fails to compile here instead of being
//! skipped silently.

use std::collections::{HashMap, HashSet};

use pycc_hir::{HirClassDef, first_definer, inherited_copy_name};

use crate::compare_chain::MirCompareKind;
use crate::exception::MirExceptionValue;
use crate::{
    CompSource, MirCompElt, MirContainerReceiver, MirExpr, MirFStringPart, MirItem, MirModule,
    MirStmt, SetElementOps, SetEqOp, SetHashOp, Ty,
};

/// Panics when a call in `module` violates the receiver-exact dispatch
/// invariant.
pub(crate) fn verify(module: &MirModule, classes: &HashMap<String, HirClassDef>) {
    let items: HashSet<&str> = module
        .items
        .iter()
        .filter_map(|item| match item {
            MirItem::Function { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    for item in &module.items {
        let (name, body): (&str, &[MirStmt]) = match item {
            MirItem::Function { name, body, .. } => (name, body),
            MirItem::TopLevelStmt(stmt) => ("<module>", std::slice::from_ref(stmt)),
            MirItem::ForeignImport { .. } => continue,
        };
        let verifier = Verifier {
            item: name,
            anchor: crate::item_anchor_class(name, classes),
            classes,
            items: &items,
        };
        verifier.stmts(body);
    }
}

struct Verifier<'a> {
    item: &'a str,
    anchor: Option<&'a str>,
    classes: &'a HashMap<String, HirClassDef>,
    items: &'a HashSet<&'a str>,
}

impl Verifier<'_> {
    fn class(&self, name: &str) -> Option<&HirClassDef> {
        self.classes.get(name)
    }

    /// The body `receiver` runs for `origin` (an item owned by
    /// `owner_class`): its copy when the module holds one, else `origin`.
    /// `spell` maps an item name to the name the call site uses (a
    /// `0gen_` protocol specialization for a generic call), since a
    /// protocol-parameter method exists only in its specialized form.
    fn body_for(
        &self,
        receiver: &HirClassDef,
        owner_class: &str,
        origin: &str,
        spell: &impl Fn(&str) -> String,
    ) -> String {
        inherited_copy_name(receiver, owner_class, origin, &|n| self.class(n))
            .filter(|copy| {
                owner_class != receiver.name
                    && (self.items.contains(copy.as_str())
                        || self.items.contains(spell(copy).as_str()))
            })
            .map_or_else(|| spell(origin), |copy| spell(&copy))
    }

    /// The callees a call of `member` (spelled with `suffix`) on a
    /// `receiver` may name: the resolved member, and -- inside a method
    /// body -- the `super()` target from the body's anchor.
    fn expected(
        &self,
        receiver: &HirClassDef,
        member: &str,
        suffix: &str,
        spell: &impl Fn(&str) -> String,
    ) -> Vec<String> {
        let mut out = Vec::new();
        let of = |n: &str| self.class(n);
        if let Some(owner) = first_definer(receiver, member, &of) {
            let origin = format!("{}.{member}{suffix}", owner.name);
            out.push(self.body_for(receiver, &owner.name, &origin, spell));
        }
        if let Some(pos) = self
            .anchor
            .and_then(|anchor| receiver.mro.iter().position(|c| c == anchor))
        {
            let after = &receiver.mro[pos + 1..];
            let find = |skip_implicit: bool| {
                after.iter().filter_map(|n| self.class(n)).find(|c| {
                    !(skip_implicit && c.implicit_object_init) && pycc_hir::binds_member(c, member)
                })
            };
            if let Some(target) = find(member == "__init__").or_else(|| find(false)) {
                let origin = format!("{}.{member}{suffix}", target.name);
                out.push(self.body_for(receiver, &target.name, &origin, spell));
            }
        }
        out
    }

    /// Checks the hash and equality callees a set of `elem_ty` elements
    /// runs on each insertion: each receives the element as `self`.
    fn check_set_ops(&self, ops: &SetElementOps, elem_ty: &Ty) {
        if let SetHashOp::Method { callee, .. } = &ops.hash {
            self.check_call(callee, elem_ty);
        }
        if let SetEqOp::Method { callee } = &ops.eq {
            self.check_call(callee, elem_ty);
        }
    }

    /// Checks one call of `callee` whose receiver has type `receiver_ty`.
    fn check_call(&self, callee: &str, receiver_ty: &Ty) {
        let Ty::Instance(receiver_name) = receiver_ty else {
            return;
        };
        let Some(receiver) = self.class(receiver_name) else {
            return;
        };
        let (generic, spelled) = match callee.strip_prefix("0gen_") {
            Some(rest) => (true, rest),
            None => (false, callee),
        };
        let Some((class, rest)) = spelled.split_once('.') else {
            return;
        };
        // A PEP 695 generic-class specialization (`0gen_C__T_int.m`) names
        // its class with the prefix, so the stripped `C__T_int` is not a
        // registered class and the call is skipped. That is deliberate: a
        // generic class never receives a receiver-exact copy (D-254 rule 2),
        // so the invariant this module guards cannot be violated there.
        if self.class(class).is_none() {
            return;
        }
        // The member spelling: `m`, `m.classmethod`, `m.setter`, `m.static`,
        // `m.0super_T[.classmethod|.setter]` (a super-target copy), and for
        // a `0gen_` specialization a `__<P>_<C>` substitution tail. The
        // kind is always the last segment.
        let (member, tail) = rest.split_once('.').unwrap_or((rest, ""));
        let (member, gen_tail) = if generic {
            split_generic_tail(member, |m| {
                pycc_hir::binds_member(receiver, m)
                    || first_definer(receiver, m, &|n| self.class(n)).is_some()
            })
        } else {
            (member, "")
        };
        if tail == "static" {
            return;
        }
        let suffix = match tail.rsplit('.').next() {
            Some(kind @ ("classmethod" | "setter")) => format!(".{kind}"),
            _ => String::new(),
        };
        let spell = |name: &str| {
            if generic {
                format!("0gen_{name}{gen_tail}")
            } else {
                name.to_string()
            }
        };
        let spelled_expected = self.expected(receiver, member, &suffix, &spell);
        assert!(
            spelled_expected.iter().any(|e| e == callee),
            "pycc_mir: internal error: receiver-exact dispatch violated (#1337, D-254): `{}` \
             calls `{callee}` with a `{receiver_name}` receiver, but that receiver resolves \
             `{member}` to {spelled_expected:?}",
            self.item
        );
    }

    fn stmts(&self, body: &[MirStmt]) {
        for stmt in body {
            self.stmt(stmt);
        }
    }

    fn exprs<'e>(&self, exprs: impl IntoIterator<Item = &'e MirExpr>) {
        for expr in exprs {
            self.expr(expr);
        }
    }

    fn source(&self, source: &CompSource) {
        match source {
            CompSource::Range { start, stop, step } => self.exprs([start, stop, step]),
            CompSource::Object(iterable) => self.expr(iterable),
            CompSource::List(_) | CompSource::Dict(_) | CompSource::Set(_) => {}
        }
    }

    fn exception(&self, value: &MirExceptionValue) {
        match value {
            MirExceptionValue::Constructed { message, .. }
            | MirExceptionValue::Existing(message) => self.expr(message),
            MirExceptionValue::ConstructedGroup {
                message, members, ..
            } => {
                self.expr(message);
                self.exprs(members);
            }
        }
    }

    fn stmt(&self, stmt: &MirStmt) {
        match stmt {
            MirStmt::ExprStmt(value)
            | MirStmt::Assign { value, .. }
            | MirStmt::ObjRaise { value } => self.expr(value),
            MirStmt::NoOp
            | MirStmt::Unreachable
            | MirStmt::Reraise
            | MirStmt::ForeignImport { .. } => {}
            MirStmt::If { test, body, orelse } => {
                self.expr(test);
                self.stmts(body);
                self.stmts(orelse);
            }
            MirStmt::While { test, body }
            | MirStmt::ForObject {
                iter: test, body, ..
            } => {
                self.expr(test);
                self.stmts(body);
            }
            MirStmt::ForRange {
                start,
                stop,
                step,
                body,
                ..
            } => {
                self.exprs([start, stop, step]);
                self.stmts(body);
            }
            MirStmt::ForList { body, .. }
            | MirStmt::ForDict { body, .. }
            | MirStmt::ForSet { body, .. }
            | MirStmt::Seq(body) => self.stmts(body),
            MirStmt::DictSet { key, value, .. }
            | MirStmt::AttrSet {
                base: key, value, ..
            } => self.exprs([key, value]),
            MirStmt::BufferSet { base, index, value } => self.exprs([base, index, value]),
            MirStmt::ObjAttrSet { base, value, .. } => self.exprs([base, value]),
            MirStmt::ObjDelAttr { base, .. } => self.expr(base),
            MirStmt::ObjDelSlice {
                base,
                start,
                stop,
                step,
            } => {
                self.expr(base);
                self.exprs([start, stop, step].into_iter().flatten());
            }
            MirStmt::ListCompAssign {
                source, cond, elt, ..
            } => {
                self.source(source);
                self.exprs(cond.as_deref());
                self.expr(elt);
            }
            MirStmt::SetCompAssign {
                source,
                cond,
                elt,
                ops,
                ..
            } => {
                self.source(source);
                self.exprs(cond.as_deref());
                self.expr(elt);
                if let Some(ops) = ops {
                    self.check_set_ops(ops, &elt.ty());
                }
            }
            MirStmt::DictCompAssign {
                source,
                cond,
                key,
                value,
                ..
            } => {
                self.source(source);
                self.exprs(cond.as_deref());
                self.exprs([key.as_ref(), value.as_ref()]);
            }
            MirStmt::Return(value) => self.exprs(value),
            MirStmt::ReturnBufferSlice { start, stop, .. } => self.exprs(start.iter().chain(stop)),
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
            } => {
                self.stmts(body);
                for handler in handlers {
                    self.stmts(&handler.body);
                }
                self.stmts(orelse);
                self.stmts(finalbody);
            }
            MirStmt::Raise { exception, .. } => self.exception(exception),
            MirStmt::RaiseFrom {
                exception, cause, ..
            } => {
                self.exception(exception);
                self.exception(cause);
            }
        }
    }

    fn container(&self, receiver: &MirContainerReceiver) {
        match receiver {
            MirContainerReceiver::Name(_) => {}
            MirContainerReceiver::Attr(expr) => self.expr(expr),
        }
    }

    fn expr(&self, expr: &MirExpr) {
        match expr {
            MirExpr::IntLiteral(_)
            | MirExpr::FloatLiteral(_)
            | MirExpr::BoolLiteral(_)
            | MirExpr::StringLiteral(_)
            | MirExpr::NoneLiteral
            | MirExpr::NotImplemented
            | MirExpr::Name { .. }
            | MirExpr::EmptyList(_)
            | MirExpr::EmptyDict(_)
            | MirExpr::NullInstance { .. }
            | MirExpr::FrozenSetFrom { source: None } => {}
            MirExpr::Call { callee, args, .. } => {
                if let Some(receiver) = args.first() {
                    self.check_call(callee, &receiver.ty());
                }
                self.exprs(args);
            }
            MirExpr::Instantiate(inst) => {
                self.check_call(&inst.ctor, &inst.ty);
                self.exprs(&inst.args);
            }
            MirExpr::IntBoundary(inner)
            | MirExpr::OptionalWrap(inner, _)
            | MirExpr::ObjectBox(inner)
            | MirExpr::OptionalUnwrap(inner, _)
            | MirExpr::ObjectUnbox(inner, _)
            | MirExpr::Not(inner)
            | MirExpr::ExceptionMessage(inner)
            | MirExpr::ExceptionTypeTest { obj: inner, .. }
            | MirExpr::AttrGet { base: inner, .. }
            | MirExpr::ObjAttrGet { base: inner, .. }
            | MirExpr::ObjLen { base: inner }
            | MirExpr::ObjType { base: inner }
            | MirExpr::BufferLen { base: inner }
            | MirExpr::BufferAlloc { len: inner }
            | MirExpr::FrozenSetFrom {
                source: Some(inner),
            }
            | MirExpr::InstanceHash { operand: inner, .. }
            | MirExpr::ObjUnpackFloatTuple { base: inner, .. }
            | MirExpr::ObjUnpack { value: inner, .. }
            | MirExpr::NamedExpr { value: inner, .. } => self.expr(inner),
            MirExpr::SetAdd { value, ops, .. } => {
                if let Some(ops) = ops {
                    self.check_set_ops(ops, &value.ty());
                }
                self.expr(value);
            }
            MirExpr::ObjIsInstance { value, class } => {
                self.expr(value);
                if let crate::ObjIsInstanceClass::Object(class) = class {
                    self.expr(class);
                }
            }
            MirExpr::BinOp { left, right, .. }
            | MirExpr::Compare { left, right, .. }
            | MirExpr::BoolOp { left, right, .. }
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
            | MirExpr::ObjCompare { left, right, .. }
            | MirExpr::ObjContains {
                item: left,
                container: right,
                ..
            }
            | MirExpr::BufferGet {
                base: left,
                index: right,
            } => self.exprs([left.as_ref(), right.as_ref()]),
            MirExpr::IfExp {
                test, body, orelse, ..
            } => self.exprs([test.as_ref(), body.as_ref(), orelse.as_ref()]),
            MirExpr::CompareChain { first, links } => {
                self.expr(first);
                for link in links {
                    if let MirCompareKind::DataclassEq { callee, .. } = &link.kind {
                        self.check_call(callee, &first.ty());
                    }
                    self.expr(&link.right);
                }
            }
            MirExpr::FString(parts) => {
                for part in parts {
                    match part {
                        MirFStringPart::Literal(_) => {}
                        MirFStringPart::Interpolation(inner) => self.expr(inner),
                    }
                }
            }
            MirExpr::ListLiteral(items)
            | MirExpr::ObjList { elements: items }
            | MirExpr::TupleLiteral(items) => self.exprs(items),
            MirExpr::SetLiteral { elements, ops } => {
                if let (Some(ops), Some(first)) = (ops, elements.first()) {
                    self.check_set_ops(ops, &first.ty());
                }
                self.exprs(elements);
            }
            MirExpr::DictLiteral(entries) => {
                for (key, value) in entries {
                    self.exprs([key, value]);
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
                self.expr(base);
                self.exprs(
                    [start, stop, step]
                        .into_iter()
                        .flatten()
                        .map(|b| b.as_ref()),
                );
            }
            MirExpr::ListAppend { list, value } => {
                self.container(list);
                self.expr(value);
            }
            MirExpr::ListPop { list, .. } => self.container(list),
            MirExpr::DictGetOrDefault {
                dict, key, default, ..
            } => {
                self.container(dict);
                self.exprs([key.as_ref(), default.as_ref()]);
            }
            MirExpr::ObjMethodCall { base, args, .. } => {
                self.expr(base);
                self.exprs(args);
            }
            MirExpr::ObjCall { callee, args } => {
                self.expr(callee);
                self.exprs(args);
            }
            MirExpr::ObjKeywordCall(call) => {
                self.expr(&call.call);
                self.exprs(&call.values);
            }
            MirExpr::Comprehension(comp) => {
                self.source(&comp.source);
                self.exprs(&comp.cond);
                match &comp.elt {
                    MirCompElt::List(elt) => self.expr(elt),
                    MirCompElt::Set(elt, ops) => {
                        self.expr(elt);
                        if let Some(ops) = ops {
                            self.check_set_ops(ops, &elt.ty());
                        }
                    }
                    MirCompElt::Dict { key, value } => self.exprs([key, value]),
                }
            }
            MirExpr::Sequence { discard, value } => self.exprs([discard.as_ref(), value.as_ref()]),
        }
    }
}

/// Splits a `0gen_` specialization's `<member>__<P>_<C>...` spelling into
/// the member and the substitution tail, choosing the split whose member
/// `is_member` accepts (a dunder member itself contains `__`).
fn split_generic_tail(spelled: &str, is_member: impl Fn(&str) -> bool) -> (&str, &str) {
    spelled
        .match_indices("__")
        .map(|(pos, _)| pos)
        .chain(std::iter::once(spelled.len()))
        .map(|pos| spelled.split_at(pos))
        .find(|(member, _)| !member.is_empty() && is_member(member))
        .unwrap_or((spelled, ""))
}

#[cfg(test)]
#[path = "verify_receiver_tests.rs"]
mod tests;
