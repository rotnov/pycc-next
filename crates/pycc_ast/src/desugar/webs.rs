//! One variable per live range: a name a scope rebinds to an unrelated value
//! (`n = input()` then `n = int(n)`; `for i in s:` then `for i in
//! range(k):`) becomes one variable per *web* -- the definitions connected
//! through the reads they reach -- so each can have its own type.
//!
//! The analysis is reaching definitions over a control-flow graph of the
//! scope's statements. Every approximation it makes over-approximates which
//! definitions reach a read (an exception may leave a `try` or `with` body
//! between any two events; a `finally` may continue to any jump target), so
//! webs are only ever merged, never wrongly split.
//!
//! A web keeps the source name when a read in it may find the name unbound
//! (its `NameError`/`UnboundLocalError`, or a builtin's fallback, names the
//! variable), or -- at module scope, where the
//! final binding is a module attribute -- when it holds the last definition.
//! Parameters and annotated names keep one variable: their declared type
//! holds for the whole scope. Names this cannot reason about are left
//! alone too: a name a nested scope
//! reads or declares `global`/`nonlocal`, one bound by an import, `def`,
//! `class`, `except ... as`, `del` or a walrus, and every name of a scope
//! that calls `locals()`, `vars()`, `globals()`, `eval()` or `exec()` or has
//! a `match` statement.

use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    ExceptHandler, Expr, ExprContext, Parameters, Stmt, StmtFunctionDef,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

mod scan;
use scan::{ScopeScan, free_names_of_class, free_names_of_def};

/// Renames every web but one of each split name in `body` (a module) and in
/// every function defined in it.
pub(crate) fn split_webs(body: &mut [Stmt]) {
    let all = all_names(body);
    let mut renames: HashMap<u32, String> = HashMap::new();
    let mut taken = all.clone();
    let mut functions = Vec::new();
    analyze_scope(&*body, &[], &mut taken, &mut renames, &mut functions);
    while let Some(def) = functions.pop() {
        let def: &StmtFunctionDef = def;
        let params = parameter_names(&def.parameters);
        analyze_scope(&def.body, &params, &mut taken, &mut renames, &mut functions);
    }
    if renames.is_empty() {
        return;
    }
    let renamer = Rename(renames);
    for stmt in body.iter_mut() {
        renamer.visit_stmt(stmt);
    }
}

struct Rename(HashMap<u32, String>);

impl Transformer for Rename {
    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Name(name) = expr
            && let Some(new) = self.0.get(&name.range.start().to_u32())
        {
            name.id = new.as_str().into();
        }
        transformer::walk_expr(self, expr);
    }
}

/// `[]`, `{}`, `set()`, `list()`, `dict()`.
fn is_empty_container(expr: &Expr) -> bool {
    match expr {
        Expr::List(list) => list.elts.is_empty(),
        Expr::Dict(dict) => dict.items.is_empty(),
        Expr::Call(call) => {
            call.arguments.args.is_empty()
                && call.arguments.keywords.is_empty()
                && matches!(call.func.as_ref(), Expr::Name(n)
                    if matches!(n.id.as_str(), "set" | "list" | "dict"))
        }
        _ => false,
    }
}

fn parameter_names(parameters: &Parameters) -> Vec<String> {
    parameters.iter().map(|p| p.name().to_string()).collect()
}

/// Every identifier `body` mentions, so a fresh name collides with none.
fn all_names(body: &[Stmt]) -> HashSet<String> {
    struct Collect(HashSet<String>);
    impl<'a> Visitor<'a> for Collect {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    self.0.insert(def.name.to_string());
                    for p in def.parameters.iter() {
                        self.0.insert(p.name().to_string());
                    }
                }
                Stmt::ClassDef(def) => {
                    self.0.insert(def.name.to_string());
                }
                Stmt::Import(import) => {
                    for alias in &import.names {
                        let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                        self.0.insert(bound.to_string());
                    }
                }
                Stmt::ImportFrom(import) => {
                    for alias in &import.names {
                        let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                        self.0.insert(bound.to_string());
                    }
                }
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Name(name) => {
                    self.0.insert(name.id.to_string());
                }
                Expr::Lambda(lambda) => {
                    if let Some(params) = &lambda.parameters {
                        for p in params.iter() {
                            self.0.insert(p.name().to_string());
                        }
                    }
                }
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut collect = Collect(HashSet::new());
    for stmt in body {
        collect.visit_stmt(stmt);
    }
    collect.0
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// A binding.
    Def,
    /// A read.
    Use,
    /// `x op= v`: a read of `x` and a binding of `x`, one name node.
    Aug,
    /// `x = []` (an empty container): a binding that stays in the web of
    /// the bindings it replaces, whose uses type its elements.
    Reset,
}

#[derive(Debug)]
struct Event {
    name: String,
    kind: Kind,
    offset: u32,
}

#[derive(Default)]
struct Block {
    events: Vec<usize>,
    succ: Vec<usize>,
}

/// The control-flow graph of one scope, built statement by statement.
struct Builder<'a> {
    blocks: Vec<Block>,
    events: Vec<Event>,
    excluded: HashSet<String>,
    cur: usize,
    exit: usize,
    /// `(continue target, break target)` of each enclosing loop.
    loops: Vec<(usize, usize)>,
    /// The blocks an exception (or a suppressed one) may continue at, per
    /// enclosing `try`/`with`.
    frames: Vec<Vec<usize>>,
    /// Functions to analyze as scopes of their own.
    functions: Vec<&'a StmtFunctionDef>,
    /// The scope cannot be reasoned about (`locals()`, `match`).
    opaque: bool,
    /// The builtins consuming a generator argument at once that the
    /// module does not rebind.
    consumers: HashSet<&'static str>,
}

/// Builtins that run a generator argument before returning: its reads
/// happen where it stands.
const CONSUMERS: [&str; 12] = [
    "any", "all", "next", "sum", "min", "max", "sorted", "list", "set", "frozenset", "tuple",
    "dict",
];

impl<'a> Builder<'a> {
    fn new_block(&mut self) -> usize {
        self.blocks.push(Block::default());
        self.blocks.len() - 1
    }

    fn edge(&mut self, from: usize, to: usize) {
        if !self.blocks[from].succ.contains(&to) {
            self.blocks[from].succ.push(to);
        }
    }

    /// Every block an exception raised now may continue at.
    fn exception_targets(&self) -> Vec<usize> {
        self.frames.iter().flatten().copied().collect()
    }

    fn push(&mut self, name: &str, kind: Kind, offset: u32) {
        self.events.push(Event {
            name: name.to_string(),
            kind,
            offset,
        });
        let index = self.events.len() - 1;
        self.blocks[self.cur].events.push(index);
        if !self.frames.is_empty() {
            // Any later event may raise: the state after this one reaches
            // the handlers.
            for target in self.exception_targets() {
                self.edge(self.cur, target);
            }
            let next = self.new_block();
            self.edge(self.cur, next);
            self.cur = next;
        }
    }

    fn uses(&mut self, expr: &'a Expr) {
        let mut walk = Uses {
            builder: self,
            shadow: Vec::new(),
        };
        walk.visit_expr(expr);
    }

    fn target(&mut self, target: &'a Expr) {
        match target {
            Expr::Name(name) => self.push(name.id.as_str(), Kind::Def, name.range.start().to_u32()),
            Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    self.target(elt);
                }
            }
            Expr::List(list) => {
                for elt in &list.elts {
                    self.target(elt);
                }
            }
            Expr::Starred(starred) => self.target(&starred.value),
            other => self.uses(other),
        }
    }

    fn stmts(&mut self, body: &'a [Stmt]) {
        for stmt in body {
            self.stmt(stmt);
        }
    }

    fn jump(&mut self, to: usize) {
        self.edge(self.cur, to);
        self.cur = self.new_block();
    }

    fn stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Expr(expr) => self.uses(&expr.value),
            Stmt::Assign(assign) => {
                self.uses(&assign.value);
                match assign.targets.as_slice() {
                    [Expr::Name(name)] if is_empty_container(&assign.value) => {
                        self.push(name.id.as_str(), Kind::Reset, name.range.start().to_u32())
                    }
                    targets => {
                        for target in targets {
                            self.target(target);
                        }
                    }
                }
            }
            Stmt::AugAssign(aug) => {
                self.uses(&aug.value);
                match aug.target.as_ref() {
                    Expr::Name(name) => {
                        self.push(name.id.as_str(), Kind::Aug, name.range.start().to_u32())
                    }
                    other => self.uses(other),
                }
            }
            Stmt::AnnAssign(ann) => {
                // A declared type is a contract for the whole scope (and
                // `Final` forbids rebinding): never split it.
                if let Expr::Name(name) = ann.target.as_ref() {
                    self.excluded.insert(name.id.to_string());
                }
                self.uses(&ann.annotation);
                if let Some(value) = &ann.value {
                    self.uses(value);
                    self.target(&ann.target);
                } else if !matches!(ann.target.as_ref(), Expr::Name(_)) {
                    self.uses(&ann.target);
                }
            }
            Stmt::For(for_stmt) => {
                self.uses(&for_stmt.iter);
                let header = self.new_block();
                let after = self.new_block();
                self.edge(self.cur, header);
                let body = self.new_block();
                self.edge(header, body);
                self.cur = body;
                self.target(&for_stmt.target);
                self.loops.push((header, after));
                self.stmts(&for_stmt.body);
                self.loops.pop();
                self.edge(self.cur, header);
                let orelse = self.new_block();
                self.edge(header, orelse);
                self.cur = orelse;
                self.stmts(&for_stmt.orelse);
                self.edge(self.cur, after);
                self.cur = after;
            }
            Stmt::While(while_stmt) => {
                let header = self.new_block();
                let after = self.new_block();
                self.edge(self.cur, header);
                self.cur = header;
                self.uses(&while_stmt.test);
                let tested = self.cur;
                let body = self.new_block();
                self.edge(tested, body);
                self.cur = body;
                self.loops.push((header, after));
                self.stmts(&while_stmt.body);
                self.loops.pop();
                self.edge(self.cur, header);
                let orelse = self.new_block();
                self.edge(tested, orelse);
                self.cur = orelse;
                self.stmts(&while_stmt.orelse);
                self.edge(self.cur, after);
                self.cur = after;
            }
            Stmt::If(if_stmt) => {
                let after = self.new_block();
                self.uses(&if_stmt.test);
                let mut tested = self.cur;
                let body = self.new_block();
                self.edge(tested, body);
                self.cur = body;
                self.stmts(&if_stmt.body);
                self.edge(self.cur, after);
                let mut has_else = false;
                for clause in &if_stmt.elif_else_clauses {
                    let block = self.new_block();
                    self.edge(tested, block);
                    self.cur = block;
                    match &clause.test {
                        Some(test) => {
                            self.uses(test);
                            tested = self.cur;
                            let body = self.new_block();
                            self.edge(tested, body);
                            self.cur = body;
                        }
                        None => has_else = true,
                    }
                    self.stmts(&clause.body);
                    self.edge(self.cur, after);
                }
                if !has_else {
                    self.edge(tested, after);
                }
                self.cur = after;
            }
            Stmt::Try(try_stmt) => {
                let after = self.new_block();
                let finally = (!try_stmt.finalbody.is_empty()).then(|| self.new_block());
                let handlers: Vec<usize> =
                    try_stmt.handlers.iter().map(|_| self.new_block()).collect();
                let mut targets = handlers.clone();
                targets.extend(finally);
                // The state on entry reaches every handler: the body
                // starts a block of its own.
                for target in &targets {
                    self.edge(self.cur, *target);
                }
                let body = self.new_block();
                self.edge(self.cur, body);
                self.cur = body;
                self.frames.push(targets);
                self.stmts(&try_stmt.body);
                self.frames.pop();
                let normal_end = finally.unwrap_or(after);
                self.frames.push(finally.into_iter().collect());
                self.stmts(&try_stmt.orelse);
                self.edge(self.cur, normal_end);
                for (handler, entry) in try_stmt.handlers.iter().zip(handlers) {
                    let ExceptHandler::ExceptHandler(handler) = handler;
                    self.cur = entry;
                    if let Some(type_) = &handler.type_ {
                        self.uses(type_);
                    }
                    if let Some(name) = &handler.name {
                        self.excluded.insert(name.to_string());
                    }
                    self.stmts(&handler.body);
                    self.edge(self.cur, normal_end);
                }
                self.frames.pop();
                if let Some(finally) = finally {
                    self.cur = finally;
                    self.stmts(&try_stmt.finalbody);
                    // A `finally` also ends a jump or an exception that
                    // left the `try`: continue at any of their targets.
                    let mut targets = vec![after, self.exit];
                    targets.extend(self.exception_targets());
                    for (cont, brk) in self.loops.clone() {
                        targets.push(cont);
                        targets.push(brk);
                    }
                    for target in targets {
                        self.edge(self.cur, target);
                    }
                }
                self.cur = after;
            }
            Stmt::With(with) => {
                let after = self.new_block();
                for item in &with.items {
                    self.uses(&item.context_expr);
                    if let Some(vars) = &item.optional_vars {
                        self.target(vars);
                    }
                }
                // A context manager may suppress an exception.
                self.edge(self.cur, after);
                let body = self.new_block();
                self.edge(self.cur, body);
                self.cur = body;
                self.frames.push(vec![after]);
                self.stmts(&with.body);
                self.frames.pop();
                self.edge(self.cur, after);
                self.cur = after;
            }
            Stmt::Return(ret) => {
                if let Some(value) = &ret.value {
                    self.uses(value);
                }
                let exit = self.exit;
                self.jump(exit);
            }
            Stmt::Raise(raise) => {
                if let Some(exc) = &raise.exc {
                    self.uses(exc);
                }
                if let Some(cause) = &raise.cause {
                    self.uses(cause);
                }
                for target in self.exception_targets() {
                    self.edge(self.cur, target);
                }
                let exit = self.exit;
                self.jump(exit);
            }
            Stmt::Break(_) => {
                if let Some(&(_, brk)) = self.loops.last() {
                    self.jump(brk);
                }
            }
            Stmt::Continue(_) => {
                if let Some(&(cont, _)) = self.loops.last() {
                    self.jump(cont);
                }
            }
            Stmt::Assert(assert) => {
                self.uses(&assert.test);
                if let Some(msg) = &assert.msg {
                    self.uses(msg);
                }
            }
            Stmt::Delete(delete) => {
                for target in &delete.targets {
                    match target {
                        Expr::Name(name) => {
                            self.excluded.insert(name.id.to_string());
                        }
                        other => self.uses(other),
                    }
                }
            }
            Stmt::Global(global) => {
                self.excluded.extend(global.names.iter().map(|n| n.to_string()));
            }
            Stmt::Nonlocal(nonlocal) => {
                self.excluded.extend(nonlocal.names.iter().map(|n| n.to_string()));
            }
            Stmt::Import(import) => {
                for alias in &import.names {
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                    self.excluded
                        .insert(bound.split('.').next().unwrap_or("").to_string());
                }
            }
            Stmt::ImportFrom(import) => {
                for alias in &import.names {
                    self.excluded
                        .insert(alias.asname.as_ref().unwrap_or(&alias.name).to_string());
                }
            }
            Stmt::FunctionDef(def) => {
                for decorator in &def.decorator_list {
                    self.uses(&decorator.expression);
                }
                for p in def.parameters.iter_non_variadic_params() {
                    if let Some(default) = &p.default {
                        self.uses(default);
                    }
                }
                self.excluded.insert(def.name.to_string());
                self.excluded.extend(free_names_of_def(def));
                self.functions.push(def);
            }
            Stmt::ClassDef(class) => {
                for decorator in &class.decorator_list {
                    self.uses(&decorator.expression);
                }
                if let Some(arguments) = &class.arguments {
                    for arg in arguments.args.iter() {
                        self.uses(arg);
                    }
                }
                self.excluded.insert(class.name.to_string());
                self.excluded.extend(free_names_of_class(class));
                for stmt in &class.body {
                    if let Stmt::FunctionDef(method) = stmt {
                        self.functions.push(method);
                    }
                }
            }
            Stmt::TypeAlias(_) | Stmt::Match(_) | Stmt::IpyEscapeCommand(_) => self.opaque = true,
            Stmt::Pass(_) => {}
        }
    }
}

/// The reads of one expression, comprehension and lambda scopes respected.
struct Uses<'b, 'a> {
    builder: &'b mut Builder<'a>,
    /// Names a comprehension binds, innermost last.
    shadow: Vec<HashSet<String>>,
}

impl<'b, 'a> Uses<'b, 'a> {
    fn comprehension(&mut self, generators: &'a [ruff_python_ast::Comprehension], elts: &[&'a Expr]) {
        let Some(first) = generators.first() else {
            return;
        };
        // The first iterable is evaluated in the enclosing scope.
        self.visit_expr(&first.iter);
        let mut bound = HashSet::new();
        for generator in generators {
            collect_targets(&generator.target, &mut bound);
        }
        self.shadow.push(bound);
        for (index, generator) in generators.iter().enumerate() {
            if index > 0 {
                self.visit_expr(&generator.iter);
            }
            for cond in &generator.ifs {
                self.visit_expr(cond);
            }
        }
        for elt in elts {
            self.visit_expr(elt);
        }
        self.shadow.pop();
    }
}

fn collect_targets(target: &Expr, out: &mut HashSet<String>) {
    match target {
        Expr::Name(name) => {
            out.insert(name.id.to_string());
        }
        Expr::Tuple(tuple) => tuple.elts.iter().for_each(|e| collect_targets(e, out)),
        Expr::List(list) => list.elts.iter().for_each(|e| collect_targets(e, out)),
        Expr::Starred(starred) => collect_targets(&starred.value, out),
        _ => {}
    }
}

impl<'b, 'a> Visitor<'a> for Uses<'b, 'a> {
    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) => {
                let id = name.id.as_str();
                if self.shadow.iter().any(|s| s.contains(id)) {
                    return;
                }
                if name.ctx == ExprContext::Load {
                    self.builder.push(id, Kind::Use, name.range.start().to_u32());
                } else {
                    // A store inside an expression (a comprehension target
                    // reached oddly): never split.
                    self.builder.excluded.insert(id.to_string());
                }
            }
            Expr::Named(named) => {
                self.visit_expr(&named.value);
                let mut targets = HashSet::new();
                collect_targets(&named.target, &mut targets);
                self.builder.excluded.extend(targets);
            }
            Expr::Lambda(lambda) => {
                let mut scan = ScopeScan::default();
                scan.visit_expr(expr);
                if let Some(parameters) = &lambda.parameters {
                    for p in parameters.iter_non_variadic_params() {
                        if let Some(default) = &p.default {
                            self.visit_expr(default);
                        }
                    }
                }
                self.builder.excluded.extend(scan.nested_free);
            }
            Expr::ListComp(comp) => self.comprehension(&comp.generators, &[&comp.elt]),
            Expr::SetComp(comp) => self.comprehension(&comp.generators, &[&comp.elt]),
            // A generator expression reads its variables when each element
            // is produced, not where it stands (all but its first
            // iterable): like a lambda's, they are never split.
            Expr::Generator(comp) => {
                self.comprehension(&comp.generators, &[&comp.elt]);
                let mut scan = ScopeScan::default();
                scan.visit_expr(&comp.elt);
                for (i, generator) in comp.generators.iter().enumerate() {
                    if i > 0 {
                        scan.visit_expr(&generator.iter);
                    }
                    for cond in &generator.ifs {
                        scan.visit_expr(cond);
                    }
                }
                self.builder.excluded.extend(scan.read);
                self.builder.excluded.extend(scan.nested_free);
            }
            Expr::DictComp(comp) => {
                let mut elts: Vec<&'a Expr> = comp.key.iter().map(|k| k.as_ref()).collect();
                elts.push(&comp.value);
                self.comprehension(&comp.generators, &elts)
            }
            Expr::Call(call) => {
                if let Expr::Name(callee) = call.func.as_ref()
                    && matches!(
                        callee.id.as_str(),
                        "locals" | "vars" | "globals" | "eval" | "exec"
                    )
                {
                    self.builder.opaque = true;
                }
                let consumer = match call.func.as_ref() {
                    Expr::Name(callee) => self.builder.consumers.contains(callee.id.as_str()),
                    Expr::Attribute(attr) => attr.attr.as_str() == "join",
                    _ => false,
                };
                self.visit_expr(&call.func);
                for arg in call.arguments.args.iter() {
                    match arg {
                        // Consumed right here: an ordinary comprehension.
                        Expr::Generator(comp) if consumer => {
                            self.comprehension(&comp.generators, &[&comp.elt])
                        }
                        _ => self.visit_expr(arg),
                    }
                }
                for keyword in call.arguments.keywords.iter() {
                    self.visit_expr(&keyword.value);
                }
            }
            _ => visitor::walk_expr(self, expr),
        }
    }
}

/// Union-find over definition ids.
struct Webs {
    parent: Vec<usize>,
}

impl Webs {
    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut walk = x;
        while self.parent[walk] != root {
            let next = self.parent[walk];
            self.parent[walk] = root;
            walk = next;
        }
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

type State = BTreeMap<String, BTreeSet<usize>>;

/// Splits the webs of one scope's body, recording renames and queueing the
/// functions it defines.
fn analyze_scope<'a>(
    body: &'a [Stmt],
    params: &[String],
    taken: &mut HashSet<String>,
    renames: &mut HashMap<u32, String>,
    functions: &mut Vec<&'a StmtFunctionDef>,
) {
    let (blocks, events, excluded, opaque, queued) = {
        let mut builder = Builder {
            blocks: Vec::new(),
            events: Vec::new(),
            excluded: HashSet::new(),
            cur: 0,
            exit: 0,
            loops: Vec::new(),
            frames: Vec::new(),
            functions: Vec::new(),
            opaque: false,
            consumers: CONSUMERS
                .iter()
                .copied()
                .filter(|name| !taken.contains(*name))
                .collect(),
        };
        let entry = builder.new_block();
        builder.exit = builder.new_block();
        builder.cur = entry;
        builder.stmts(body);
        let (cur, exit) = (builder.cur, builder.exit);
        builder.edge(cur, exit);
        (
            builder.blocks,
            builder.events,
            builder.excluded,
            builder.opaque,
            builder.functions,
        )
    };
    functions.extend(queued);
    if opaque {
        return;
    }
    // Candidate names: bound here, not excluded, not a dunder, not a
    // parameter (its annotation types it for the whole body).
    let candidates: BTreeSet<&str> = events
        .iter()
        .filter(|e| matches!(e.kind, Kind::Def | Kind::Aug | Kind::Reset))
        .map(|e| e.name.as_str())
        .filter(|n| {
            !excluded.contains(*n) && !n.starts_with("__") && !params.iter().any(|p| p == n)
        })
        .collect();
    if candidates.is_empty() {
        return;
    }
    // Definition ids: one per binding event, then one "unbound" pseudo
    // definition and one per parameter at entry.
    let mut def_of_event: HashMap<usize, usize> = HashMap::new();
    let mut def_event: Vec<Option<usize>> = Vec::new();
    for (index, event) in events.iter().enumerate() {
        if matches!(event.kind, Kind::Def | Kind::Aug | Kind::Reset) && candidates.contains(event.name.as_str()) {
            def_of_event.insert(index, def_event.len());
            def_event.push(Some(index));
        }
    }
    let mut entry: State = State::new();
    let mut must_keep: HashSet<usize> = HashSet::new();
    let mut unbound: HashSet<usize> = HashSet::new();
    let mut param_defs: Vec<(&str, usize)> = Vec::new();
    for name in &candidates {
        let id = def_event.len();
        def_event.push(None);
        if params.iter().any(|p| p == name) {
            must_keep.insert(id);
            param_defs.push((name, id));
        } else {
            unbound.insert(id);
        }
        entry.insert(name.to_string(), BTreeSet::from([id]));
    }
    let transfer = |state: &mut State, event_index: usize| {
        let event = &events[event_index];
        if let Some(def) = def_of_event.get(&event_index) {
            state.insert(event.name.clone(), BTreeSet::from([*def]));
        }
    };
    // Reaching definitions to a fixpoint.
    let mut ins: Vec<Option<State>> = vec![None; blocks.len()];
    ins[0] = Some(entry);
    let mut work: Vec<usize> = vec![0];
    while let Some(block) = work.pop() {
        let Some(mut state) = ins[block].clone() else {
            continue;
        };
        for &event in &blocks[block].events {
            transfer(&mut state, event);
        }
        for &succ in &blocks[block].succ {
            let changed = match &mut ins[succ] {
                None => {
                    ins[succ] = Some(state.clone());
                    true
                }
                Some(existing) => {
                    let mut changed = false;
                    for (name, defs) in &state {
                        let slot = existing.entry(name.clone()).or_default();
                        for def in defs {
                            changed |= slot.insert(*def);
                        }
                    }
                    changed
                }
            };
            if changed {
                work.push(succ);
            }
        }
    }
    // Webs: the reads join the definitions reaching them.
    let mut webs = Webs {
        parent: (0..def_event.len()).collect(),
    };
    let mut use_web: Vec<(usize, usize)> = Vec::new(); // (event, a def it reads)
    let mut unbound_read: Vec<usize> = Vec::new(); // defs reached alongside "unbound"
    for (block, state) in blocks.iter().zip(&ins) {
        let Some(mut state) = state.clone() else {
            continue;
        };
        for &event_index in &block.events {
            let event = &events[event_index];
            if matches!(event.kind, Kind::Use | Kind::Aug | Kind::Reset)
                && let Some(defs) = state.get(&event.name)
            {
                let real: Vec<usize> = defs.iter().copied().filter(|d| !unbound.contains(d)).collect();
                let maybe_unbound = defs.iter().any(|d| unbound.contains(d));
                let mut joined = real.clone();
                if matches!(event.kind, Kind::Aug | Kind::Reset) {
                    joined.push(def_of_event[&event_index]);
                }
                for pair in joined.windows(2) {
                    webs.union(pair[0], pair[1]);
                }
                if let Some(first) = joined.first() {
                    if event.kind == Kind::Use {
                        use_web.push((event_index, *first));
                    }
                    if maybe_unbound && event.kind != Kind::Reset {
                        unbound_read.push(*first);
                    }
                }
            }
            transfer(&mut state, event_index);
        }
    }
    let mut keep_roots: HashSet<usize> = HashSet::new();
    for def in must_keep.iter().chain(&unbound_read) {
        keep_roots.insert(webs.find(*def));
    }
    // Group each candidate's real definitions by web.
    let mut by_name: BTreeMap<&str, BTreeMap<usize, Vec<usize>>> = BTreeMap::new();
    for (def, event) in def_event.iter().enumerate() {
        if let Some(event) = event {
            let root = webs.find(def);
            by_name
                .entry(events[*event].name.as_str())
                .or_default()
                .entry(root)
                .or_default()
                .push(def);
        }
    }
    // A parameter's web is one of its name's webs even with no binding
    // event in the body.
    for (name, def) in &param_defs {
        let root = webs.find(*def);
        by_name.entry(name).or_default().entry(root).or_default();
    }
    let mut rename_root: HashMap<usize, String> = HashMap::new();
    for (name, groups) in by_name {
        if groups.len() < 2 {
            continue;
        }
        let kept: Vec<usize> = groups
            .keys()
            .copied()
            .filter(|root| keep_roots.contains(root))
            .collect();
        let keep = match kept.as_slice() {
            [one] => *one,
            [] => {
                // The last binding keeps the name: at module scope it is
                // the module attribute's final value.
                let last = groups
                    .iter()
                    .max_by_key(|(_, defs)| {
                        defs.iter()
                            .map(|d| events[def_event[*d].expect("real")].offset)
                            .max()
                    })
                    .map(|(root, _)| *root)
                    .expect("two groups");
                last
            }
            _ => continue,
        };
        // A dead `x = []` stays with the kept variable: alone it would be an
        // empty container nothing types.
        let dead_reset = |root: &usize, defs: &Vec<usize>, webs: &mut Webs| {
            defs.iter()
                .all(|d| def_event[*d].is_some_and(|e| events[e].kind == Kind::Reset))
                && !use_web.iter().any(|(_, def)| webs.find(*def) == *root)
        };
        let renamed: Vec<usize> = groups
            .iter()
            .filter(|(root, defs)| **root != keep && !dead_reset(root, defs, &mut webs))
            .map(|(root, _)| *root)
            .collect();
        for root in renamed {
            let mut k = 1;
            let fresh = loop {
                let candidate = format!("{name}_{k}");
                if !taken.contains(&candidate) {
                    break candidate;
                }
                k += 1;
            };
            taken.insert(fresh.clone());
            rename_root.insert(root, fresh);
        }
    }
    if rename_root.is_empty() {
        return;
    }
    for (def, event) in def_event.iter().enumerate() {
        if let Some(event) = event
            && let Some(new) = rename_root.get(&webs.find(def))
        {
            renames.insert(events[*event].offset, new.clone());
        }
    }
    for (event, def) in use_web {
        if let Some(new) = rename_root.get(&webs.find(def)) {
            renames.insert(events[event].offset, new.clone());
        }
    }
}

#[cfg(test)]
mod tests;
