//! The prover of a region's native types (docs/design/selected-regions.md,
//! section 4.2): which parameters of each member's copy are native `int`s,
//! which are exact `str`s (fact B1), and whether its result is an `int` or a
//! tuple whose items are `int`s or objects.
//!
//! One type per local, solved to a fixpoint over the region. A parameter's
//! kind is the meet of what every region call site passes it; a local's
//! type the join of its bindings; a result the join of the `return`s (and
//! `None` where the body may run off its end). Parameter kinds only fall
//! and results only rise while the fixpoint is solved, so it ends.
//!
//! What starts a chain of `int`s without a check that could fail:
//!
//! - an `int` literal, `len()` of anything, and `+`, `-`, `*` of `int`s;
//! - B1: `s.index(...)`, `s.find(...)`, `s.rfind(...)`, `s.rindex(...)` and
//!   `s.count(...)` of an exact `str` `s` (a parameter every entry guards
//!   and every region call site passes an exact `str`, or a value proven
//!   one: `s.replace(...)` of one, a subscript of one);
//! - B2: a call of a local every binding of which reads one of the
//!   members' module globals, through the copies (`super::rewrite`);
//! - B3: `m.start()` and `m.end()` of `m = P.match(...)` (or `search`,
//!   `fullmatch`) when `P` is a module global the region watches, which
//!   holds an exact `re.Pattern` whenever the region is intact;
//! - a region call of a member whose copy's result is proven.
//!
//! A local any nested scope reads or binds is an object (section 4.8).

use super::{Edge, Member};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, Number, Operator, Stmt, UnaryOp};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// The kind of a copy's parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Not constrained yet (no region call site seen).
    Top,
    Int,
    /// An exact `str`, passed as the object it is.
    Str,
    Obj,
}

impl Kind {
    fn meet(self, other: Kind) -> Kind {
        match (self, other) {
            (Kind::Top, k) | (k, Kind::Top) => k,
            (a, b) if a == b => a,
            _ => Kind::Obj,
        }
    }
}

/// The type of a value in a member's body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ty {
    /// No binding seen yet (or a call whose result is not known yet).
    Bot,
    Int,
    Str,
    /// The result of `match`, `search` or `fullmatch` of one of these
    /// pattern globals.
    Match(BTreeSet<String>),
    Obj,
    Tuple(Vec<Ty>),
}

impl Ty {
    pub(crate) fn join(&self, other: &Ty) -> Ty {
        match (self, other) {
            (Ty::Bot, t) | (t, Ty::Bot) => t.clone(),
            (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
                Ty::Tuple(a.iter().zip(b).map(|(x, y)| x.join(y)).collect())
            }
            (Ty::Match(a), Ty::Match(b)) => Ty::Match(a.union(b).cloned().collect()),
            (a, b) if a == b => a.clone(),
            _ => Ty::Obj,
        }
    }

    fn kind(&self) -> Kind {
        match self {
            Ty::Bot => Kind::Top,
            Ty::Int => Kind::Int,
            Ty::Str => Kind::Str,
            _ => Kind::Obj,
        }
    }

    /// The shape a copy may return: `int`, a tuple of `int`s and objects
    /// with at least one `int`, or an object. While the fixpoint runs, an
    /// item not known yet stays `Bot` (`settled` makes it an object).
    pub(crate) fn result(&self) -> Ty {
        match self {
            Ty::Int => Ty::Int,
            Ty::Tuple(items) if items.len() >= 2 && items.iter().any(|t| matches!(t, Ty::Int | Ty::Bot)) => {
                Ty::Tuple(
                    items
                        .iter()
                        .map(|t| match t {
                            Ty::Int | Ty::Bot => t.clone(),
                            _ => Ty::Obj,
                        })
                        .collect(),
                )
            }
            Ty::Bot => Ty::Bot,
            _ => Ty::Obj,
        }
    }

    /// Whether `self` may still become `other`: equal where both are known.
    fn agrees(&self, other: &Ty) -> bool {
        match (self, other) {
            (Ty::Bot, _) | (_, Ty::Bot) => true,
            (Ty::Tuple(a), Ty::Tuple(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.agrees(y)),
            (a, b) => a == b,
        }
    }

    /// The result once the fixpoint has settled: what is still unknown is
    /// an object.
    fn settled(&self) -> Ty {
        match self {
            Ty::Bot => Ty::Obj,
            Ty::Tuple(items) => Ty::Tuple(items.iter().map(Ty::settled).collect()).result(),
            t => t.clone(),
        }
    }
}

/// A member's signature: its parameters' kinds and its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sig {
    pub(crate) params: Vec<Kind>,
    pub(crate) result: Ty,
}

/// What one pass over a member's body found.
struct Found {
    /// The parameters that must be objects: rebound to another type, or
    /// read by a nested scope.
    demoted: BTreeSet<usize>,
    result: Ty,
    /// Each region call's argument types, by callee and parameter.
    passed: Vec<(usize, Vec<Ty>)>,
    /// Whether a local is a native `int`.
    native_local: bool,
    /// The pattern globals whose matches' `start()` or `end()` it takes as
    /// proven `int`s (B3): the ones the region must watch.
    patterns: BTreeSet<String>,
}

/// The methods of an exact `str` whose result is an `int` whenever they
/// return, and those whose result is an exact `str`.
const STR_INT_METHODS: [&str; 5] = ["index", "find", "rfind", "rindex", "count"];
const STR_STR_METHODS: [&str; 6] = ["replace", "lower", "upper", "strip", "lstrip", "rstrip"];
const MATCH_METHODS: [&str; 3] = ["match", "search", "fullmatch"];

/// The signatures of `members`, which call each other through `edges`
/// (`super::Edges`), or None when no fixpoint is reached.
pub(crate) fn prove(members: &[Member<'_>], edges: &super::Edges<'_>) -> Option<Vec<Sig>> {
    let mut called: BTreeSet<usize> = BTreeSet::new();
    for sites in edges.sites.values() {
        for (_, edge) in sites {
            called.extend(edge.callees());
        }
    }
    let mut sigs: Vec<Sig> = members
        .iter()
        .enumerate()
        .map(|(at, m)| Sig {
            params: m
                .params
                .iter()
                .map(|p| {
                    if called.contains(&at) {
                        Kind::Top
                    } else if demands_str(&m.def.body, p) {
                        Kind::Str
                    } else {
                        Kind::Obj
                    }
                })
                .collect(),
            result: Ty::Bot,
        })
        .collect();
    for _ in 0..64 {
        let found: Vec<Found> = (0..members.len()).map(|at| analyze(at, members, edges, &sigs)).collect();
        let mut next: Vec<Sig> = sigs
            .iter()
            .enumerate()
            .map(|(at, sig)| Sig {
                params: if called.contains(&at) { vec![Kind::Top; sig.params.len()] } else { sig.params.clone() },
                result: found[at].result.result(),
            })
            .collect();
        for f in &found {
            for (callee, args) in &f.passed {
                for (param, arg) in next[*callee].params.iter_mut().zip(args) {
                    *param = param.meet(arg.kind());
                }
            }
        }
        for (at, f) in found.iter().enumerate() {
            for p in &f.demoted {
                next[at].params[*p] = Kind::Obj;
            }
        }
        // Parameter kinds only fall: a call site the last round typed
        // more precisely is not trusted to raise them back.
        for (n, s) in next.iter_mut().zip(&sigs) {
            for (a, b) in n.params.iter_mut().zip(&s.params) {
                *a = b.meet(*a);
            }
        }
        if next == sigs {
            // Settle what is still unknown, and run again until settling
            // changes nothing: an object result may lower a caller's kinds.
            let settled: Vec<Sig> = sigs
                .iter()
                .map(|s| Sig {
                    params: s.params.iter().map(|k| if *k == Kind::Top { Kind::Obj } else { *k }).collect(),
                    result: s.result.settled(),
                })
                .collect();
            if settled == sigs {
                return Some(sigs);
            }
            sigs = settled;
            continue;
        }
        sigs = next;
    }
    None
}

/// The pattern globals member `at`'s copy relies on (B3): those whose
/// matches' `start()` or `end()` it takes as proven `int`s with `sigs`.
pub(crate) fn proven_patterns(at: usize, members: &[Member<'_>], edges: &super::Edges<'_>, sigs: &[Sig]) -> BTreeSet<String> {
    analyze(at, members, edges, sigs).patterns
}

/// Whether member `at`'s body gives its copy anything native: a native
/// parameter, result or local.
pub(crate) fn useful(at: usize, members: &[Member<'_>], edges: &super::Edges<'_>, sigs: &[Sig]) -> bool {
    let sig = &sigs[at];
    sig.params.iter().any(|k| matches!(k, Kind::Int | Kind::Str))
        || sig.result != Ty::Obj
        || analyze(at, members, edges, sigs).native_local
}

/// Whether `param` is used as an exact `str` would be: the receiver of a
/// `str` method B1 types.
fn demands_str(body: &[Stmt], param: &str) -> bool {
    struct Demand<'p> {
        param: &'p str,
        found: bool,
    }
    impl<'a> Visitor<'a> for Demand<'_> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Call(call) => {
                    if let Expr::Attribute(attr) = call.func.as_ref()
                        && matches!(attr.value.as_ref(), Expr::Name(n) if n.id.as_str() == self.param)
                        && (STR_INT_METHODS.contains(&attr.attr.as_str())
                            || STR_STR_METHODS.contains(&attr.attr.as_str()))
                    {
                        self.found = true;
                    }
                }
                Expr::Lambda(_) => return,
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
                visitor::walk_stmt(self, stmt);
            }
        }
    }
    let mut demand = Demand { param, found: false };
    for stmt in body {
        demand.visit_stmt(stmt);
    }
    demand.found
}

/// One binding of a local, as the analysis evaluates it.
enum Binding<'a> {
    Value(&'a Expr),
    /// Item `index` of a tuple value of `arity` items.
    Item(&'a Expr, usize, usize),
    /// `name op= value`.
    Aug(&'a str, Operator, &'a Expr),
    /// Anything else (a `for` target, an `except` name, an import, ...).
    Opaque,
}

/// The pass over member `at`'s body with signatures `sigs`.
fn analyze(at: usize, members: &[Member<'_>], edges: &super::Edges<'_>, sigs: &[Sig]) -> Found {
    let member = &members[at];
    let sig = &sigs[at];
    let mut bindings: Vec<(&str, Binding<'_>)> = Vec::new();
    let mut returns: Vec<Option<&Expr>> = Vec::new();
    collect(&member.def.body, &mut bindings, &mut returns);
    let mut types: BTreeMap<&str, Ty> = BTreeMap::new();
    for (p, kind) in member.params.iter().zip(&sig.params) {
        types.insert(
            p.as_str(),
            match kind {
                Kind::Int => Ty::Int,
                Kind::Str => Ty::Str,
                Kind::Top => Ty::Bot,
                Kind::Obj => Ty::Obj,
            },
        );
    }
    let captured = &member.captured;
    let patterns = RefCell::new(BTreeSet::new());
    let env_ty = |types: &BTreeMap<&str, Ty>, e: &Expr| {
        let env = Env { at, members, edges, sigs, types, patterns: &patterns };
        env.ty(e)
    };
    for _ in 0..32 {
        let mut next = types.clone();
        for (name, binding) in &bindings {
            let t = if captured.contains(*name) {
                Ty::Obj
            } else {
                match binding {
                    Binding::Value(e) => env_ty(&types, e),
                    // A copy returns a tuple as a CPython object, whose `int`
                    // items only an unpack of the call itself takes
                    // (`rewrite::split_unpacks`); a tuple display is native.
                    Binding::Item(e, i, n) => match (e, env_ty(&types, e)) {
                        (Expr::Call(_) | Expr::Tuple(_), Ty::Tuple(items)) if items.len() == *n => items[*i].clone(),
                        (_, Ty::Bot) => Ty::Bot,
                        _ => Ty::Obj,
                    },
                    Binding::Aug(target, op, e) => {
                        let left = types.get(target).cloned().unwrap_or(Ty::Bot);
                        arith(*op, &left, &env_ty(&types, e))
                    }
                    Binding::Opaque => Ty::Obj,
                }
            };
            let slot = next.entry(name).or_insert(Ty::Bot);
            *slot = slot.join(&t);
        }
        for name in captured {
            if let Some(slot) = next.get_mut(name.as_str()) {
                *slot = Ty::Obj;
            }
        }
        if next == types {
            break;
        }
        types = next;
    }
    let mut demoted = BTreeSet::new();
    for (i, (p, kind)) in member.params.iter().zip(&sig.params).enumerate() {
        let t = types.get(p.as_str()).cloned().unwrap_or(Ty::Obj);
        let keeps = match kind {
            Kind::Int => t == Ty::Int,
            Kind::Str => t == Ty::Str,
            _ => true,
        };
        if !keeps || captured.contains(p) {
            demoted.insert(i);
        }
    }
    // What the last round proved is what the copy relies on.
    patterns.borrow_mut().clear();
    for (name, binding) in &bindings {
        if !captured.contains(*name) {
            match binding {
                Binding::Value(e) | Binding::Item(e, ..) | Binding::Aug(_, _, e) => {
                    env_ty(&types, e);
                }
                Binding::Opaque => {}
            }
        }
    }
    let env = Env { at, members, edges, sigs, types: &types, patterns: &patterns };
    let mut result = if super::terminates(&member.def.body) { Ty::Bot } else { Ty::Obj };
    for r in &returns {
        let t = match r {
            Some(e) => env.ty(e),
            None => Ty::Obj,
        };
        result = result.join(&t);
    }
    let mut passed: Vec<(usize, Vec<Ty>)> = Vec::new();
    if let Some(sites) = edges.sites.get(&at) {
        for (call, edge) in sites {
            if !edges.binds(call, edge) {
                continue;
            }
            for callee in edge.callees() {
                if let Some(args) = edges.bound_args(call, callee) {
                    passed.push((callee, args.iter().map(|a| env.ty(a)).collect()));
                }
            }
        }
    }
    let native_local = types
        .iter()
        .any(|(name, t)| *t == Ty::Int && !member.params.iter().any(|p| p == name));
    Found { demoted, result, passed, native_local, patterns: patterns.into_inner() }
}

/// `left op right` of two types.
fn arith(op: Operator, left: &Ty, right: &Ty) -> Ty {
    match (op, left, right) {
        (Operator::Add | Operator::Sub | Operator::Mult, Ty::Int, Ty::Int) => Ty::Int,
        (_, Ty::Bot, _) | (_, _, Ty::Bot) => Ty::Bot,
        _ => Ty::Obj,
    }
}

/// The bindings and `return`s of a body, not those of its nested scopes.
fn collect<'a>(body: &'a [Stmt], bindings: &mut Vec<(&'a str, Binding<'a>)>, returns: &mut Vec<Option<&'a Expr>>) {
    for stmt in body {
        collect_stmt(stmt, bindings, returns);
    }
}

fn target<'a>(t: &'a Expr, value: Option<&'a Expr>, bindings: &mut Vec<(&'a str, Binding<'a>)>) {
    match t {
        Expr::Name(n) => bindings.push((
            n.id.as_str(),
            match value {
                Some(v) => Binding::Value(v),
                None => Binding::Opaque,
            },
        )),
        Expr::Tuple(tuple) => {
            let n = tuple.elts.len();
            for (i, elt) in tuple.elts.iter().enumerate() {
                match (elt, value) {
                    (Expr::Name(name), Some(v)) => bindings.push((name.id.as_str(), Binding::Item(v, i, n))),
                    _ => target(elt, None, bindings),
                }
            }
        }
        Expr::List(list) => {
            for elt in &list.elts {
                target(elt, None, bindings);
            }
        }
        Expr::Starred(s) => target(&s.value, None, bindings),
        _ => {}
    }
}

fn collect_stmt<'a>(stmt: &'a Stmt, bindings: &mut Vec<(&'a str, Binding<'a>)>, returns: &mut Vec<Option<&'a Expr>>) {
    // Walrus targets inside expressions are opaque.
    struct Walrus<'b, 'a> {
        bindings: &'b mut Vec<(&'a str, Binding<'a>)>,
    }
    impl<'a> Visitor<'a> for Walrus<'_, 'a> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Named(named) => {
                    if let Expr::Name(n) = named.target.as_ref() {
                        self.bindings.push((n.id.as_str(), Binding::Opaque));
                    }
                    visitor::walk_expr(self, expr);
                }
                Expr::Lambda(_) | Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {}
                _ => visitor::walk_expr(self, expr),
            }
        }
        fn visit_stmt(&mut self, _stmt: &'a Stmt) {}
    }
    let walrus = |e: &'a Expr, bindings: &mut Vec<(&'a str, Binding<'a>)>| {
        Walrus { bindings }.visit_expr(e);
    };
    match stmt {
        Stmt::Assign(a) => {
            walrus(&a.value, bindings);
            for t in &a.targets {
                target(t, Some(&a.value), bindings);
            }
        }
        Stmt::AnnAssign(a) => {
            if let Some(v) = &a.value {
                walrus(v, bindings);
            }
            let int_annotation = matches!(a.annotation.as_ref(), Expr::Name(n) if n.id.as_str() == "int");
            match (a.target.as_ref(), &a.value) {
                (Expr::Name(n), Some(v)) if int_annotation => bindings.push((n.id.as_str(), Binding::Value(v))),
                (t, _) => target(t, None, bindings),
            }
        }
        Stmt::AugAssign(a) => {
            walrus(&a.value, bindings);
            match a.target.as_ref() {
                Expr::Name(n) => bindings.push((n.id.as_str(), Binding::Aug(n.id.as_str(), a.op, &a.value))),
                t => target(t, None, bindings),
            }
        }
        Stmt::Return(r) => {
            if let Some(v) = &r.value {
                walrus(v, bindings);
            }
            returns.push(r.value.as_deref());
        }
        Stmt::For(f) => {
            target(&f.target, None, bindings);
            collect(&f.body, bindings, returns);
            collect(&f.orelse, bindings, returns);
        }
        Stmt::While(w) => {
            walrus(&w.test, bindings);
            collect(&w.body, bindings, returns);
            collect(&w.orelse, bindings, returns);
        }
        Stmt::If(i) => {
            walrus(&i.test, bindings);
            collect(&i.body, bindings, returns);
            for clause in &i.elif_else_clauses {
                if let Some(test) = &clause.test {
                    walrus(test, bindings);
                }
                collect(&clause.body, bindings, returns);
            }
        }
        Stmt::With(w) => {
            for item in &w.items {
                if let Some(v) = &item.optional_vars {
                    target(v, None, bindings);
                }
            }
            collect(&w.body, bindings, returns);
        }
        Stmt::Try(t) => {
            collect(&t.body, bindings, returns);
            for handler in &t.handlers {
                let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                if let Some(name) = &h.name {
                    bindings.push((name.as_str(), Binding::Opaque));
                }
                collect(&h.body, bindings, returns);
            }
            collect(&t.orelse, bindings, returns);
            collect(&t.finalbody, bindings, returns);
        }
        // A member with a `match` statement gets no copy (`super::eligible`).
        Stmt::Match(_) => {}
        Stmt::FunctionDef(def) => bindings.push((def.name.as_str(), Binding::Opaque)),
        Stmt::ClassDef(class) => bindings.push((class.name.as_str(), Binding::Opaque)),
        Stmt::Import(i) => {
            for alias in &i.names {
                let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                bindings.push((bound.as_str().split('.').next().unwrap_or(""), Binding::Opaque));
            }
        }
        Stmt::ImportFrom(i) => {
            for alias in &i.names {
                let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                bindings.push((bound.as_str(), Binding::Opaque));
            }
        }
        Stmt::Delete(d) => {
            for t in &d.targets {
                target(t, None, bindings);
            }
        }
        Stmt::Expr(e) => walrus(&e.value, bindings),
        _ => {}
    }
}

/// The context [`Env::ty`] types an expression of member `at` in.
struct Env<'e, 'a> {
    at: usize,
    members: &'e [Member<'a>],
    edges: &'e super::Edges<'a>,
    sigs: &'e [Sig],
    types: &'e BTreeMap<&'a str, Ty>,
    /// Where [`Env::call`] records the pattern globals behind each match
    /// whose `start()` or `end()` it types `int`.
    patterns: &'e RefCell<BTreeSet<String>>,
}

impl Env<'_, '_> {
    fn ty(&self, e: &Expr) -> Ty {
        match e {
            Expr::NumberLiteral(n) if matches!(n.value, Number::Int(_)) => Ty::Int,
            Expr::StringLiteral(_) => Ty::Str,
            // A local not bound yet in this round is not known yet.
            Expr::Name(n) => match self.types.get(n.id.as_str()) {
                Some(t) => t.clone(),
                None if self.members[self.at].locals.contains(n.id.as_str()) => Ty::Bot,
                None => Ty::Obj,
            },
            Expr::BinOp(b) => arith(b.op, &self.ty(&b.left), &self.ty(&b.right)),
            Expr::UnaryOp(u) if u.op == UnaryOp::USub => match self.ty(&u.operand) {
                Ty::Int => Ty::Int,
                Ty::Bot => Ty::Bot,
                _ => Ty::Obj,
            },
            Expr::If(i) => self.ty(&i.body).join(&self.ty(&i.orelse)),
            Expr::Tuple(t) if !t.elts.iter().any(|e| matches!(e, Expr::Starred(_))) => {
                Ty::Tuple(t.elts.iter().map(|e| self.ty(e)).collect())
            }
            Expr::Subscript(s) => match self.ty(&s.value) {
                Ty::Str => Ty::Str,
                Ty::Bot => Ty::Bot,
                _ => Ty::Obj,
            },
            Expr::Call(call) => self.call(call),
            _ => Ty::Obj,
        }
    }

    fn call(&self, call: &ruff_python_ast::ExprCall) -> Ty {
        let member = &self.members[self.at];
        if let Some(edge) = self.edges.edge(self.at, call) {
            if !self.edges.binds(call, edge) {
                return Ty::Obj;
            }
            let mut t = Ty::Bot;
            for callee in edge.callees() {
                t = t.join(&self.sigs[callee].result);
            }
            // A dispatch's copies must agree (`super::rewrite`); one not
            // known yet agrees for now.
            if let Edge::Dispatch(targets) = edge
                && targets
                    .iter()
                    .any(|c| !self.sigs[*c].result.agrees(&t))
            {
                return Ty::Obj;
            }
            return t;
        }
        let args = &call.arguments;
        match call.func.as_ref() {
            Expr::Name(n)
                if n.id.as_str() == "len"
                    && !member.binds("len")
                    && args.args.len() == 1
                    && args.keywords.is_empty()
                    && !matches!(args.args[0], Expr::Starred(_)) =>
            {
                Ty::Int
            }
            Expr::Attribute(attr) => {
                let receiver = self.ty(&attr.value);
                let method = attr.attr.as_str();
                let plain = !args.args.iter().any(|a| matches!(a, Expr::Starred(_)))
                    && args.keywords.iter().all(|k| k.arg.is_some());
                match receiver {
                    Ty::Str if plain && STR_INT_METHODS.contains(&method) => Ty::Int,
                    Ty::Str if plain && STR_STR_METHODS.contains(&method) => Ty::Str,
                    // Not known yet: may still be a `str` or a match.
                    Ty::Bot
                        if plain
                            && (STR_INT_METHODS.contains(&method)
                                || STR_STR_METHODS.contains(&method)
                                || matches!(method, "start" | "end")) =>
                    {
                        Ty::Bot
                    }
                    Ty::Match(from)
                        if matches!(method, "start" | "end") && args.args.is_empty() && args.keywords.is_empty() =>
                    {
                        self.patterns.borrow_mut().extend(from);
                        Ty::Int
                    }
                    _ if MATCH_METHODS.contains(&method) && self.edges.pattern(&attr.value).is_some() => {
                        Ty::Match(self.edges.pattern(&attr.value).into_iter().map(str::to_string).collect())
                    }
                    _ => Ty::Obj,
                }
            }
            _ => Ty::Obj,
        }
    }
}
