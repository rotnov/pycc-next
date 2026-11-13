//! Native calls inside a region of selected functions
//! (docs/design/selected-regions.md, R2).
//!
//! The compile unit of a module that selects functions holds each selected
//! function's *lift*: a module-level function whose parameters and result
//! are objects, reading every module global from the namespace
//! (`__pycc_fallback_ns['name']`). This pass gives the members of a region
//! *copies* with native signatures:
//!
//! ```python
//! def __pycc_lift13_skip_chars__(src: object, pos: object, chars: object) -> object:
//!     if __pycc_region_intact(0) and type(src) is __pycc_specialize_str and type(pos) is __pycc_specialize_int:
//!         return __pycc_rnative_lift13_skip_chars__(src, pos, chars)
//!     ...                                   # the generic body, unchanged
//! def __pycc_rnative_lift13_skip_chars__(src: object, pos: int, chars: object) -> int:
//!     ...                                   # the same body
//! ```
//!
//! and calls a member's copy from another copy in place of the object call
//! through the namespace, while the region is intact:
//!
//! ```python
//! pos = (__pycc_rnative_lift13_skip_chars__(src, pos, TOML_WS) if __pycc_region_intact(0)
//!        else __pycc_rslow0(__pycc_fallback_ns['skip_chars'], 'skip_chars() returned ', src, pos, TOML_WS))
//! ```
//!
//! The generic arm is a shared, cold *slow call* (`slow`), and the copy's
//! own call of a member returning a tuple is unpacked where it is made, so
//! it takes the callee's multi-value twin; so does an entry guard's, which
//! then builds the tuple. A copy nothing calls then is not emitted
//! (`pycc_mir::unreferenced`), and the lift of a member with a copy takes no
//! multi-value twin of its own (`pycc_mir::multi_return_object`): its
//! generic body runs only once the region is closed, or for arguments of
//! other types.
//!
//! A region is *intact* while every member's module global holds the
//! function the trailer armed it with, running its code and defaults, and
//! every pattern global a copy relies on (B3) holds the object it held
//! (`src/ext/module/regions.c`). The flag is cleared, never set again, by a
//! watcher; an entry and every region call check it. So a copy runs only on
//! values its proof covers: the conversion on the generic arm of a call is
//! reached only when a member was rebound while a region call was running
//! (the DEVIATIONS row), and nowhere else is an object converted.
//!
//! What a copy's types are is [`prove::prove`]'s. A member gets a copy only
//! when it gives something native (a parameter, its result or a local);
//! the others stay generic lifts, and calls of them stay object calls.
//! Method edges stay generic (section 4.3).

use super::super::build::{call_name, int, load, string, tuple};
use super::Specialized;
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, ExprCall, ExprContext, ModModule, Stmt, StmtFunctionDef};
use ruff_text_size::TextRange;
use std::collections::{BTreeMap, BTreeSet};

mod prove;
mod rewrite;
mod slow;

pub use slow::SLOW_PREFIX as REGION_SLOW_PREFIX;

use prove::{Kind, Ty};

/// The name of the namespace dict the lifts read module globals from.
const NS: &str = "__pycc_fallback_ns";

/// The prefix of a copy's name: the lift's name follows, without its
/// leading `__pycc_`.
pub const REGION_COPY_PREFIX: &str = "__pycc_rnative_";

/// The most regions one module arms: the length of the module unit's flag
/// array (`PYCC_EXT_REGION_MAX`, `src/ext/module/module_ctx.c`, which a
/// `pycc_codegen` test holds to this value). Regions past it get no copies.
pub const REGION_MAX: usize = 64;

/// A compiled module function that a region edge may call: its lift, the
/// module global CPython binds it to (once, with its own undecorated
/// `def`), its report name, and how many of its parameters a call may pass
/// positionally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionFunction {
    pub lift: String,
    pub global: String,
    pub report: String,
    /// How many leading parameters are positional-only.
    pub posonly: usize,
    pub positional: usize,
    /// Each parameter's default, when it is a literal (its source text).
    pub defaults: Vec<Option<String>>,
}

/// What the selected unit tells this pass: the functions region edges may
/// call, and the module globals bound once (by an assignment or an import)
/// a copy may rely on holding an exact `re.Pattern` (B3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegionInput {
    pub functions: Vec<RegionFunction>,
    pub patterns: Vec<String>,
}

/// One member, as the prover sees it.
pub(crate) struct Member<'a> {
    pub(crate) function: &'a RegionFunction,
    pub(crate) def: &'a StmtFunctionDef,
    pub(crate) params: Vec<String>,
    /// The names its body binds.
    pub(crate) locals: BTreeSet<String>,
    /// The names a nested scope of its body reads or binds.
    pub(crate) captured: BTreeSet<String>,
}

impl Member<'_> {
    /// Whether `name` is a parameter or a local of the member, so that a
    /// bare `name` in its body is not the builtin of that name.
    pub(crate) fn binds(&self, name: &str) -> bool {
        self.locals.contains(name) || self.params.iter().any(|p| p == name)
    }
}

/// How a region call names its callee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Edge {
    /// `__pycc_fallback_ns['g'](...)`.
    Direct(usize),
    /// `f(...)`, `f` a local every binding of which reads one of these
    /// members' globals (B2).
    Dispatch(Vec<usize>),
}

impl Edge {
    pub(crate) fn callees(&self) -> Vec<usize> {
        match self {
            Edge::Direct(c) => vec![*c],
            Edge::Dispatch(cs) => cs.clone(),
        }
    }
}

/// Where an argument of a region call comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgRef {
    Positional(usize),
    Keyword(usize),
    /// The callee's literal default of this parameter.
    Default(usize),
}

/// The region calls of the members, by caller.
pub(crate) struct Edges<'a> {
    pub(crate) sites: BTreeMap<usize, Vec<(&'a ExprCall, Edge)>>,
    /// Each call's arguments in its callee's parameter order, when they
    /// bind every parameter in source order.
    bound: BTreeMap<(*const ExprCall, usize), Vec<ArgRef>>,
    patterns: &'a [String],
    /// Each member's literal defaults, parsed.
    defaults: Vec<Vec<Option<Expr>>>,
}

impl<'a> Edges<'a> {
    pub(crate) fn edge(&self, at: usize, call: &ExprCall) -> Option<&Edge> {
        self.sites
            .get(&at)?
            .iter()
            .find(|(c, _)| std::ptr::eq(*c, call))
            .map(|(_, e)| e)
    }

    /// Whether the call binds its callee's every parameter, and the
    /// arguments in parameter order.
    pub(crate) fn bound_args<'s>(&'s self, call: &'s ExprCall, callee: usize) -> Option<Vec<&'s Expr>> {
        let refs = self.bound.get(&(call as *const ExprCall, callee))?;
        refs.iter()
            .map(|r| match r {
                ArgRef::Positional(i) => Some(&call.arguments.args[*i]),
                ArgRef::Keyword(i) => Some(&call.arguments.keywords[*i].value),
                ArgRef::Default(p) => self.defaults[callee][*p].as_ref(),
            })
            .collect()
    }

    /// Whether `call` binds every one of its callees' parameters.
    pub(crate) fn binds(&self, call: &ExprCall, edge: &Edge) -> bool {
        edge.callees()
            .iter()
            .all(|c| self.bound.contains_key(&(call as *const ExprCall, *c)))
    }

    /// The watched pattern global `expr` reads, if it reads one.
    pub(crate) fn pattern(&self, expr: &Expr) -> Option<&str> {
        let name = global_read(expr)?;
        self.patterns.iter().find(|p| p.as_str() == name).map(String::as_str)
    }
}


/// `name` when `expr` is `__pycc_fallback_ns['name']`.
pub(crate) fn global_read(expr: &Expr) -> Option<&str> {
    let Expr::Subscript(sub) = expr else {
        return None;
    };
    let (Expr::Name(ns), Expr::StringLiteral(key)) = (sub.value.as_ref(), sub.slice.as_ref()) else {
        return None;
    };
    (ns.id.as_str() == NS).then(|| key.value.to_str())
}

/// Whether running `body` to its end is impossible: its last statement
/// returns, raises, or is a loop or branch that cannot run off its end.
pub(crate) fn terminates(body: &[Stmt]) -> bool {
    let Some(last) = body.last() else {
        return false;
    };
    match last {
        Stmt::Return(_) | Stmt::Raise(_) => true,
        Stmt::If(i) => {
            terminates(&i.body)
                && i.elif_else_clauses.iter().any(|c| c.test.is_none())
                && i.elif_else_clauses.iter().all(|c| terminates(&c.body))
        }
        Stmt::While(w) => matches!(w.test.as_ref(), Expr::BooleanLiteral(b) if b.value) && !breaks(&w.body),
        Stmt::Try(t) => {
            terminates(&t.finalbody)
                || ((terminates(&t.body) || terminates(&t.orelse))
                    && t.handlers.iter().all(|h| {
                        let ruff_python_ast::ExceptHandler::ExceptHandler(h) = h;
                        terminates(&h.body)
                    }))
        }
        Stmt::With(w) => terminates(&w.body),
        _ => false,
    }
}

/// Whether a `break` of the loop whose body is `body` is in it.
fn breaks(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        Stmt::Break(_) => true,
        Stmt::If(i) => breaks(&i.body) || i.elif_else_clauses.iter().any(|c| breaks(&c.body)),
        Stmt::Try(t) => {
            breaks(&t.body)
                || breaks(&t.orelse)
                || breaks(&t.finalbody)
                || t.handlers.iter().any(|h| {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = h;
                    breaks(&h.body)
                })
        }
        Stmt::With(w) => breaks(&w.body),
        Stmt::Match(m) => m.cases.iter().any(|c| breaks(&c.body)),
        _ => false,
    })
}

/// Whether a member's lift can get a copy: plain positional parameters, no
/// `yield`, `match`, `global` or `nonlocal`.
fn eligible(def: &StmtFunctionDef) -> bool {
    struct Plain(bool);
    impl<'a> Visitor<'a> for Plain {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::Match(_) | Stmt::Global(_) | Stmt::Nonlocal(_) => self.0 = false,
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => return,
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Yield(_) | Expr::YieldFrom(_) | Expr::Await(_) => self.0 = false,
                Expr::Lambda(_) | Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {
                    return;
                }
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
    }
    let p = &def.parameters;
    if def.is_async
        || !def.decorator_list.is_empty()
        || p.vararg.is_some()
        || p.kwarg.is_some()
        || !p.posonlyargs.is_empty()
        || !p.kwonlyargs.is_empty()
        || p.args.iter().any(|a| a.default.is_some())
    {
        return false;
    }
    let mut plain = Plain(true);
    for stmt in &def.body {
        plain.visit_stmt(stmt);
    }
    plain.0
}

/// The names the nested scopes of `body` read or bind.
fn captured(body: &[Stmt]) -> BTreeSet<String> {
    struct Names(BTreeSet<String>);
    impl<'a> Visitor<'a> for Names {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(n) = expr {
                self.0.insert(n.id.to_string());
            }
            visitor::walk_expr(self, expr);
        }
    }
    struct Nested(BTreeSet<String>);
    impl<'a> Visitor<'a> for Nested {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                    let mut names = Names(BTreeSet::new());
                    visitor::walk_stmt(&mut names, stmt);
                    self.0.extend(names.0);
                }
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Lambda(_) | Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {
                    let mut names = Names(BTreeSet::new());
                    names.visit_expr(expr);
                    self.0.extend(names.0);
                }
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    let mut nested = Nested(BTreeSet::new());
    for stmt in body {
        nested.visit_stmt(stmt);
    }
    nested.0
}

/// The region calls of `members`' bodies.
fn find_edges<'a>(members: &[Member<'a>], patterns: &'a [String]) -> Edges<'a> {
    let by_global: BTreeMap<&str, usize> = members
        .iter()
        .enumerate()
        .map(|(at, m)| (m.function.global.as_str(), at))
        .collect();
    struct Calls<'a, 'b> {
        by_global: &'b BTreeMap<&'b str, usize>,
        dispatch: &'b BTreeMap<String, Vec<usize>>,
        found: Vec<(&'a ExprCall, Edge)>,
    }
    impl<'a> Visitor<'a> for Calls<'a, '_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
                visitor::walk_stmt(self, stmt);
            }
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Lambda(_) | Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {
                    return;
                }
                Expr::Call(call) => {
                    if let Some(g) = global_read(&call.func)
                        && let Some(c) = self.by_global.get(g)
                    {
                        self.found.push((call, Edge::Direct(*c)));
                    } else if let Expr::Name(n) = call.func.as_ref()
                        && let Some(targets) = self.dispatch.get(n.id.as_str())
                    {
                        self.found.push((call, Edge::Dispatch(targets.clone())));
                    }
                }
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut sites = BTreeMap::new();
    let mut bound = BTreeMap::new();
    for (at, m) in members.iter().enumerate() {
        let dispatch = dispatch_locals(m, &by_global);
        let mut calls = Calls { by_global: &by_global, dispatch: &dispatch, found: Vec::new() };
        for stmt in &m.def.body {
            calls.visit_stmt(stmt);
        }
        for (call, edge) in &calls.found {
            let callees = edge.callees();
            let binds: Vec<Option<Vec<ArgRef>>> = callees.iter().map(|c| bind(call, &members[*c])).collect();
            if binds.iter().all(Option::is_some) {
                for (c, b) in callees.iter().zip(binds) {
                    bound.insert((*call as *const ExprCall, *c), b.expect("checked"));
                }
            }
        }
        if !calls.found.is_empty() {
            sites.insert(at, calls.found);
        }
    }
    let defaults = members
        .iter()
        .map(|m| {
            m.function
                .defaults
                .iter()
                .map(|d| {
                    let text = d.as_ref()?;
                    let parsed = ruff_python_parser::parse_expression(text).ok()?;
                    Some(parsed.into_expr())
                })
                .collect()
        })
        .collect();
    Edges { sites, bound, patterns, defaults }
}

/// `call`'s arguments in `callee`'s parameter order: when they bind each
/// parameter once, in source order (so the copy evaluates them in the order
/// CPython does), with no `*` or `**`.
fn bind(call: &ExprCall, callee: &Member<'_>) -> Option<Vec<ArgRef>> {
    let args = &call.arguments;
    let f = callee.function;
    if args.args.iter().any(|a| matches!(a, Expr::Starred(_))) || args.args.len() > f.positional {
        return None;
    }
    let mut refs: Vec<Option<ArgRef>> = vec![None; callee.params.len()];
    for (i, r) in refs.iter_mut().enumerate().take(args.args.len()) {
        *r = Some(ArgRef::Positional(i));
    }
    let mut last = args.args.len();
    for (k, keyword) in args.keywords.iter().enumerate() {
        let name = keyword.arg.as_ref()?;
        let at = callee.params.iter().position(|p| p == name.as_str())?;
        if at < last.max(f.posonly) || refs[at].is_some() {
            return None;
        }
        refs[at] = Some(ArgRef::Keyword(k));
        last = at + 1;
    }
    refs.iter()
        .enumerate()
        .map(|(p, r)| match r {
            Some(r) => Some(*r),
            None => f.defaults.get(p)?.as_ref().map(|_| ArgRef::Default(p)),
        })
        .collect()
}

/// The locals of `member` every binding of which reads one of the region's
/// member globals (`f = __pycc_fallback_ns['g']`), with those members.
fn dispatch_locals(member: &Member<'_>, by_global: &BTreeMap<&str, usize>) -> BTreeMap<String, Vec<usize>> {
    struct Reads<'b> {
        by_global: &'b BTreeMap<&'b str, usize>,
        targets: BTreeMap<String, Option<Vec<usize>>>,
    }
    impl<'a> Visitor<'a> for Reads<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if let Stmt::Assign(a) = stmt
                && let [Expr::Name(n)] = a.targets.as_slice()
            {
                let target = global_read(&a.value).and_then(|g| self.by_global.get(g).copied());
                let slot = self.targets.entry(n.id.to_string()).or_insert(Some(Vec::new()));
                match (slot.as_mut(), target) {
                    (Some(list), Some(t)) => {
                        if !list.contains(&t) {
                            list.push(t);
                        }
                    }
                    _ => *slot = None,
                }
                return;
            }
            if !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
                visitor::walk_stmt(self, stmt);
            }
        }
    }
    let mut reads = Reads { by_global, targets: BTreeMap::new() };
    for stmt in &member.def.body {
        reads.visit_stmt(stmt);
    }
    // Any other binding of the name (a parameter, a `for` target, ...)
    // disqualifies it.
    let mut other: BTreeSet<String> = member.params.iter().cloned().collect();
    other.extend(member.captured.iter().cloned());
    for stmt in &member.def.body {
        collect_other_bindings(stmt, &mut other);
    }
    reads
        .targets
        .into_iter()
        .filter_map(|(name, t)| {
            let t = t?;
            (!t.is_empty() && !other.contains(&name)).then_some((name, t))
        })
        .collect()
}

/// The names `stmt` binds other than by a plain `name = value`.
fn collect_other_bindings(stmt: &Stmt, out: &mut BTreeSet<String>) {
    match stmt {
        Stmt::Assign(a) if matches!(a.targets.as_slice(), [Expr::Name(_)]) => {
            let mut inner = crate::namespace::bound_names(std::slice::from_ref(stmt));
            if let [Expr::Name(n)] = a.targets.as_slice() {
                inner.remove(n.id.as_str());
            }
            out.extend(inner);
        }
        Stmt::If(i) => {
            out.extend(walrus_names(&i.test));
            for s in &i.body {
                collect_other_bindings(s, out);
            }
            for c in &i.elif_else_clauses {
                for s in &c.body {
                    collect_other_bindings(s, out);
                }
            }
        }
        Stmt::While(w) => {
            out.extend(walrus_names(&w.test));
            for s in w.body.iter().chain(&w.orelse) {
                collect_other_bindings(s, out);
            }
        }
        Stmt::Try(t) => {
            for s in t.body.iter().chain(&t.orelse).chain(&t.finalbody) {
                collect_other_bindings(s, out);
            }
            for h in &t.handlers {
                let ruff_python_ast::ExceptHandler::ExceptHandler(h) = h;
                if let Some(n) = &h.name {
                    out.insert(n.to_string());
                }
                for s in &h.body {
                    collect_other_bindings(s, out);
                }
            }
        }
        Stmt::With(w) => {
            for item in &w.items {
                if let Some(v) = &item.optional_vars {
                    out.extend(crate::namespace::target_names(v));
                }
            }
            for s in &w.body {
                collect_other_bindings(s, out);
            }
        }
        Stmt::For(f) => {
            out.extend(crate::namespace::target_names(&f.target));
            for s in f.body.iter().chain(&f.orelse) {
                collect_other_bindings(s, out);
            }
        }
        _ => out.extend(crate::namespace::bound_names(std::slice::from_ref(stmt))),
    }
}

fn walrus_names(expr: &Expr) -> BTreeSet<String> {
    struct W(BTreeSet<String>);
    impl<'a> Visitor<'a> for W {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Named(n) = expr {
                self.0.extend(crate::namespace::target_names(&n.target));
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut w = W(BTreeSet::new());
    w.visit_expr(expr);
    w.0
}

/// The connected components of the edges among `n` members that have an
/// edge, each in member order.
fn components(n: usize, edges: &Edges<'_>) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut u: usize) -> usize {
        while parent[u] != u {
            parent[u] = parent[parent[u]];
            u = parent[u];
        }
        u
    }
    let mut touched = BTreeSet::new();
    for (caller, sites) in &edges.sites {
        for (_, edge) in sites {
            for callee in edge.callees() {
                touched.insert(*caller);
                touched.insert(callee);
                let (a, b) = (find(&mut parent, *caller), find(&mut parent, callee));
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for m in touched {
        let root = find(&mut parent, m);
        by_root.entry(root).or_default().push(m);
    }
    by_root.into_values().collect()
}

/// The members of `region` that can reach themselves through its edges.
fn cyclic(region: &[usize], edges: &Edges<'_>) -> BTreeSet<usize> {
    let succ = |m: usize| -> Vec<usize> {
        edges
            .sites
            .get(&m)
            .map(|sites| sites.iter().flat_map(|(_, e)| e.callees()).collect())
            .unwrap_or_default()
    };
    let mut out = BTreeSet::new();
    for &m in region {
        let mut seen = BTreeSet::new();
        let mut stack = succ(m);
        while let Some(x) = stack.pop() {
            if x == m {
                out.insert(m);
                break;
            }
            if seen.insert(x) {
                stack.extend(succ(x));
            }
        }
    }
    out
}

/// Adds the regions' copies and entry guards to `module`, the selected
/// unit, for the functions of `input` that `skip` does not name (by report
/// name), and returns one [`Specialized`] per copy, its `qualname` the
/// member's report name after `region `.
pub fn specialize_regions(
    module: &mut ModModule,
    input: &RegionInput,
    skip: &dyn Fn(&str) -> bool,
) -> Vec<Specialized> {
    let plan = settle(module, input, skip, |members, edges, regions, sigs| {
        rewrite::plan(members, edges, regions, sigs, &|r| cyclic(r, edges))
    });
    match plan {
        Some(plan) => rewrite::apply(module, plan),
        None => Vec::new(),
    }
}

/// The lifts of `input`'s functions whose copies in `module`, the selected
/// unit, return a native position: an `int`, or a tuple whose first item
/// is one. A function with no copy is not among them.
///
/// The selected unit asks this of its position-flow callees
/// (docs/design/selected-regions.md, section 4.1): one joins its region
/// only when its result is proven native.
#[must_use]
pub fn region_native_results(module: &ModModule, input: &RegionInput) -> BTreeSet<String> {
    settle(module, input, &|_| false, |members, _, _, sigs| {
        members
            .iter()
            .zip(sigs)
            .filter(|(_, sig)| match &sig.result {
                Ty::Int => true,
                Ty::Tuple(items) => items.first() == Some(&Ty::Int),
                _ => false,
            })
            .map(|(member, _)| member.function.lift.clone())
            .collect()
    })
    .unwrap_or_default()
}

/// Proves the regions of `module`'s functions of `input` that `skip` does
/// not name, dropping members without a useful copy until every one left
/// has one, and hands the members, edges, regions and signatures to
/// `finish`. `None` when no member is left or the proof fails.
fn settle<R>(
    module: &ModModule,
    input: &RegionInput,
    skip: &dyn Fn(&str) -> bool,
    finish: impl FnOnce(&[Member<'_>], &Edges<'_>, &[Vec<usize>], &[prove::Sig]) -> R,
) -> Option<R> {
    if input.functions.is_empty()
        || ["type", "int", "str"].iter().any(|b| crate::namespace::module_scope_names(&module.body).contains(*b))
    {
        return None;
    }
    let defs: BTreeMap<&str, &StmtFunctionDef> = module
        .body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::FunctionDef(def) => Some((def.name.as_str(), def)),
            _ => None,
        })
        .collect();
    let mut chosen: Vec<&RegionFunction> = input
        .functions
        .iter()
        .filter(|f| !skip(&format!("region {}", f.report)))
        .filter(|f| defs.get(f.lift.as_str()).is_some_and(|d| eligible(d)))
        .collect();
    loop {
        let members: Vec<Member<'_>> = chosen
            .iter()
            .map(|f| {
                let def = defs[f.lift.as_str()];
                Member {
                    function: f,
                    def,
                    params: def.parameters.args.iter().map(|a| a.parameter.name.to_string()).collect(),
                    locals: crate::namespace::bound_names(&def.body),
                    captured: captured(&def.body),
                }
            })
            .collect();
        let edges = find_edges(&members, &input.patterns);
        let regions = components(members.len(), &edges);
        let sigs = prove::prove(&members, &edges)?;
        let in_region: BTreeSet<usize> = regions.iter().flatten().copied().collect();
        let keep: Vec<usize> = (0..members.len())
            .filter(|m| in_region.contains(m) && prove::useful(*m, &members, &edges, &sigs))
            .collect();
        if keep.len() == members.len() {
            return Some(finish(&members, &edges, &regions, &sigs));
        }
        if keep.is_empty() {
            return None;
        }
        chosen = keep.iter().map(|m| chosen[*m]).collect();
    }
}

/// `__pycc_region_intact(k)`.
pub(crate) fn intact(k: usize, range: TextRange) -> Expr {
    call_name("__pycc_region_intact", vec![int(k as u64, range)], range)
}

/// `__pycc_fallback_ns["__pycc_regions__"](k, (members...), (patterns...))`.
pub(crate) fn arm(k: usize, members: &[&str], patterns: &[&str], range: TextRange) -> Stmt {
    let names = |names: &[&str]| tuple(names.iter().map(|n| string(n, range)).collect(), ExprContext::Load, range);
    let hook = super::super::build::subscript(load(NS, range), string("__pycc_regions__", range), ExprContext::Load, range);
    Stmt::Expr(ruff_python_ast::StmtExpr {
        node_index: ruff_python_ast::AtomicNodeIndex::default(),
        range,
        value: Box::new(super::super::build::call(
            hook,
            vec![int(k as u64, range), names(members), names(patterns)],
            range,
        )),
    })
}

/// A copy's annotation of kind `k`.
pub(crate) fn kind_annotation(k: Kind) -> &'static str {
    if k == Kind::Int { "int" } else { "object" }
}

/// A copy's result annotation, and the suffix of the conversion its
/// generic arm takes (`None`: an object, no conversion).
pub(crate) fn result_annotation(t: &Ty) -> (String, Option<String>) {
    match t {
        Ty::Int => ("int".to_string(), Some("i".to_string())),
        // Unpacked where it is made, the call takes the copy's multi-value
        // twin and builds no tuple (`rewrite::split_sites`).
        Ty::Tuple(items) => (
            format!(
                "tuple[{}]",
                items
                    .iter()
                    .map(|t| if *t == Ty::Int { "int" } else { "object" })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Some(items.iter().map(|t| if *t == Ty::Int { 'i' } else { 'o' }).collect()),
        ),
        _ => ("object".to_string(), None),
    }
}

