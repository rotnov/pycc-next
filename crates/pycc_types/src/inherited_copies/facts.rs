//! What a method body does with its receiver (#1337, D-254): the syntactic
//! facts the copy-needed fixpoint in [`super::plan`] reads.
//!
//! The walk is a conservative over-approximation. Every sub-expression and
//! sub-statement is visited, including lambda-free nested shapes such as
//! comprehensions and f-string interpolations, and a receiver use that is
//! not recognised as an attribute base, a `super()` access, an
//! `isinstance`/`issubclass` subject, a `cls(...)` callee or a
//! `type(self)(...)` construction counts as a
//! *bare* use. Over-approximating can only add a copy (or turn an escape
//! into an honest refusal), never lose one.

use std::collections::BTreeSet;

use pycc_hir::{
    CompIter, ContainerReceiver, FStringPart, HirExpr, HirPattern, HirStmt, extract_class_names,
};

/// The receiver-related facts of one method body.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct BodyFacts {
    /// Members named through the receiver: `self.x`, `self.x(...)`,
    /// `self.x = v` (and `cls.x` in a classmethod).
    pub(crate) self_refs: BTreeSet<String>,
    /// Members named through `super()`: `super().x`, `super().x(...)`.
    pub(crate) super_refs: BTreeSet<String>,
    /// Classes named by `isinstance(self, T)` / `issubclass(cls, T)`.
    pub(crate) receiver_type_tests: BTreeSet<String>,
    /// Whether the body constructs through its receiver (`cls(...)` or
    /// `type(self)(...)`).
    pub(crate) constructs_via_receiver: bool,
    /// Whether the receiver is used other than as an attribute base, a
    /// `super()` access, a type-test subject, or a constructor callee.
    pub(crate) bare_use: bool,
}

impl BodyFacts {
    /// Merges another occurrence's facts (a method redefined in one class
    /// body has several items under one name).
    pub(crate) fn merge(&mut self, other: BodyFacts) {
        self.self_refs.extend(other.self_refs);
        self.super_refs.extend(other.super_refs);
        self.receiver_type_tests.extend(other.receiver_type_tests);
        self.constructs_via_receiver |= other.constructs_via_receiver;
        self.bare_use |= other.bare_use;
    }
}

/// Collects [`BodyFacts`] for a body whose receiver parameter is named
/// `receiver`, and -- as a side product -- every class a program-wide
/// identity observer names (`observers`).
pub(crate) struct Walker<'a> {
    receiver: Option<&'a str>,
    pub(crate) facts: BodyFacts,
    pub(crate) observers: BTreeSet<String>,
}

impl<'a> Walker<'a> {
    /// A walker for a method body with receiver `receiver`, or for a
    /// non-method item (`None`) where only observers are collected.
    pub(crate) fn new(receiver: Option<&'a str>) -> Self {
        Walker {
            receiver,
            facts: BodyFacts::default(),
            observers: BTreeSet::new(),
        }
    }

    fn is_receiver(&self, expr: &HirExpr) -> bool {
        matches!(expr, HirExpr::Name(name) if Some(name.as_str()) == self.receiver)
    }

    fn name_use(&mut self, name: &str) {
        if Some(name) == self.receiver {
            self.facts.bare_use = true;
        }
    }

    pub(crate) fn stmts(&mut self, body: &[HirStmt]) {
        for stmt in body {
            self.stmt(stmt);
        }
    }

    fn comp_iter(&mut self, iter: &CompIter) {
        match iter {
            CompIter::Range { start, stop, step } => {
                self.expr(start);
                self.expr(stop);
                self.expr(step);
            }
            CompIter::Iterable(iterable) => self.expr(iterable),
            CompIter::Name(name) => self.name_use(name),
        }
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::ExprStmt(expr) => self.expr(expr),
            HirStmt::Assign { target, value } => {
                self.name_use(target);
                self.expr(value);
            }
            HirStmt::AnnAssign { target, value, .. } => {
                self.name_use(target);
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            HirStmt::If { test, body, orelse } => {
                self.expr(test);
                self.stmts(body);
                self.stmts(orelse);
            }
            HirStmt::While { test, body } => {
                self.expr(test);
                self.stmts(body);
            }
            HirStmt::ForRange {
                var,
                start,
                stop,
                step,
                body,
            } => {
                self.name_use(var);
                self.expr(start);
                self.expr(stop);
                self.expr(step);
                self.stmts(body);
            }
            HirStmt::ForList { var, list, body } => {
                self.name_use(var);
                self.name_use(list);
                self.stmts(body);
            }
            HirStmt::ForObject { var, iter, body } => {
                self.name_use(var);
                self.expr(iter);
                self.stmts(body);
            }
            HirStmt::DictSet { dict, key, value } => {
                self.name_use(dict);
                self.expr(key);
                self.expr(value);
            }
            HirStmt::ListCompAssign {
                target,
                var,
                iter,
                cond,
                elt,
            }
            | HirStmt::SetCompAssign {
                target,
                var,
                iter,
                cond,
                elt,
            } => {
                self.name_use(target);
                self.name_use(var);
                self.comp_iter(iter);
                if let Some(cond) = cond {
                    self.expr(cond);
                }
                self.expr(elt);
            }
            HirStmt::DictCompAssign {
                target,
                var,
                iter,
                cond,
                key,
                value,
            } => {
                self.name_use(target);
                self.name_use(var);
                self.comp_iter(iter);
                if let Some(cond) = cond {
                    self.expr(cond);
                }
                self.expr(key);
                self.expr(value);
            }
            HirStmt::Return(value) => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            HirStmt::AttrSet { base, attr, value } => {
                if self.is_receiver(base) {
                    self.facts.self_refs.insert(attr.clone());
                } else {
                    self.expr(base);
                }
                self.expr(value);
            }
            HirStmt::Match { subject, cases } => {
                self.expr(subject);
                for case in cases {
                    self.pattern(&case.pattern);
                    if let Some(guard) = &case.guard {
                        self.expr(guard);
                    }
                    self.stmts(&case.body);
                }
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
                self.stmts(body);
                for handler in handlers {
                    self.observers
                        .extend(handler.exc_type.iter().flatten().cloned());
                    if let Some(name) = &handler.name {
                        self.name_use(name);
                    }
                    self.stmts(&handler.body);
                }
                self.stmts(orelse);
                self.stmts(finalbody);
            }
            HirStmt::Raise { exc, cause } => {
                for expr in [exc, cause].into_iter().flatten() {
                    self.expr(expr);
                }
            }
            HirStmt::Delete { name } => self.name_use(name),
            HirStmt::DeleteSlice {
                base,
                start,
                stop,
                step,
                ..
            } => {
                self.expr(base);
                for bound in [start, stop, step].into_iter().flatten() {
                    self.expr(bound);
                }
            }
            HirStmt::DeleteAttr { base, .. } => self.expr(base),
            HirStmt::ForeignImport { .. } => {}
        }
    }

    fn pattern(&mut self, pattern: &HirPattern) {
        match pattern {
            HirPattern::Wildcard | HirPattern::Singleton(_) | HirPattern::NoneSingleton => {}
            HirPattern::Capture(name) => self.name_use(name),
            HirPattern::Literal(expr) => self.expr(expr),
            HirPattern::Sequence(items) | HirPattern::Or(items) => {
                for item in items {
                    self.pattern(item);
                }
            }
            HirPattern::SequenceStar(items, rest) => {
                for item in items {
                    self.pattern(item);
                }
                if let Some(rest) = rest {
                    self.name_use(rest);
                }
            }
            HirPattern::Mapping(entries, rest) => {
                for (key, value) in entries {
                    self.expr(key);
                    self.pattern(value);
                }
                if let Some(rest) = rest {
                    self.name_use(rest);
                }
            }
            HirPattern::Class {
                class_name,
                positional,
                keyword,
            } => {
                self.observers.insert(class_name.clone());
                for item in positional {
                    self.pattern(item);
                }
                for (_, item) in keyword {
                    self.pattern(item);
                }
            }
            HirPattern::As(inner, name) => {
                self.pattern(inner);
                self.name_use(name);
            }
        }
    }

    fn container(&mut self, receiver: &ContainerReceiver) {
        match receiver {
            ContainerReceiver::Name(name) => self.name_use(name),
            ContainerReceiver::Attr(expr) => self.expr(expr),
        }
    }

    fn expr(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::IntLiteral(_)
            | HirExpr::FloatLiteral(_)
            | HirExpr::BoolLiteral(_)
            | HirExpr::StringLiteral(_)
            | HirExpr::NoneLiteral
            | HirExpr::NotImplemented
            | HirExpr::EmptyList(_)
            | HirExpr::EmptyDict(_)
            | HirExpr::Super => {}
            HirExpr::Name(name) => self.name_use(name),
            HirExpr::Call { callee, args } => self.call(callee, args),
            HirExpr::BinOp { left, right, .. }
            | HirExpr::Compare { left, right, .. }
            | HirExpr::BoolOp { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            HirExpr::IfExp { test, body, orelse } => {
                for part in [test, body, orelse] {
                    self.expr(part);
                }
            }
            HirExpr::CompareChain { first, links } => {
                self.expr(first);
                for link in links {
                    self.expr(&link.right);
                }
            }
            HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
                self.expr(operand)
            }
            HirExpr::FString(parts) => {
                for part in parts {
                    if let FStringPart::Interpolation(expr) = part {
                        self.expr(expr);
                    }
                }
            }
            HirExpr::ListLiteral(items)
            | HirExpr::ObjectList(items)
            | HirExpr::SetLiteral(items)
            | HirExpr::TupleLiteral(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            HirExpr::DictLiteral(entries) => {
                for (key, value) in entries {
                    self.expr(key);
                    self.expr(value);
                }
            }
            HirExpr::Subscript { base, index } => {
                self.expr(base);
                self.expr(index);
            }
            HirExpr::Slice {
                base,
                start,
                stop,
                step,
            } => {
                self.expr(base);
                for bound in [start, stop, step].into_iter().flatten() {
                    self.expr(bound);
                }
            }
            HirExpr::ListAppend { list, value } => {
                self.container(list);
                self.expr(value);
            }
            HirExpr::ListPop { list } => self.container(list),
            HirExpr::DictGetOrDefault { dict, key, default } => {
                self.container(dict);
                self.expr(key);
                self.expr(default);
            }
            HirExpr::SetAdd { set, value } => {
                self.name_use(set);
                self.expr(value);
            }
            HirExpr::AttrGet { base, attr } => self.member_base(base, attr),
            HirExpr::MethodCall { base, method, args } => {
                self.member_base(base, method);
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExpr::ReceiverDispatchedCall { call, .. } => self.expr(call),
            HirExpr::KeywordCall { call, keywords, .. } => {
                self.expr(call);
                for (_, value) in keywords {
                    self.expr(value);
                }
            }
            HirExpr::GenericClassInstantiate { args, .. } => {
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExpr::ExprCall { callee, args } => {
                self.expr(callee);
                for arg in args {
                    self.expr(arg);
                }
            }
            // #1411: `type(self)(...)` constructs through the receiver
            // exactly as `cls(...)` does, so a subclass needs its own copy.
            // A body walked without a receiver (an item that is not an
            // instance or class method, such as a `@staticmethod` whose
            // `self` parameter is an ordinary one) gets no copy from it.
            HirExpr::ReceiverClassCall { args } => {
                if self.receiver.is_some() {
                    self.facts.constructs_via_receiver = true;
                }
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExpr::NamedExpr { name, value } => {
                self.name_use(name);
                self.expr(value);
            }
            HirExpr::Comprehension(comp) => {
                self.name_use(&comp.var);
                for sub in comp.sub_exprs() {
                    self.expr(sub);
                }
                if let CompIter::Name(name) = &comp.iter {
                    self.name_use(name);
                }
            }
        }
    }

    fn member_base(&mut self, base: &HirExpr, member: &str) {
        if self.is_receiver(base) {
            self.facts.self_refs.insert(member.to_string());
        } else if matches!(base, HirExpr::Super) {
            self.facts.super_refs.insert(member.to_string());
        } else {
            self.expr(base);
        }
    }

    fn call(&mut self, callee: &str, args: &[HirExpr]) {
        if Some(callee) == self.receiver {
            self.facts.constructs_via_receiver = true;
        }
        if matches!(callee, "isinstance" | "issubclass") && args.len() == 2 {
            let targets = extract_class_names(&args[1]).unwrap_or_default();
            self.observers.extend(targets.iter().cloned());
            if self.is_receiver(&args[0]) {
                self.facts.receiver_type_tests.extend(targets);
            } else {
                self.expr(&args[0]);
            }
            return;
        }
        for arg in args {
            self.expr(arg);
        }
    }
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod tests;
