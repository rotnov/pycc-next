//! Expressions that become calls of synthesized module-level helpers.
//!
//! * A list, set or dict comprehension with more than one `for` clause (or
//!   more than one `if` clause)
//!   (`[x * y for x in xs for y in ys if x != y]`) becomes a call of a
//!   helper that builds the result with nested loops:
//!
//!   ```python
//!   def _pycc_comp_N(ys, xs):
//!       _pycc_r = []
//!       for x in xs:
//!           for y in ys:
//!               if x != y:
//!                   _pycc_r.append(x * y)
//!       return _pycc_r
//!   ```
//!
//!   A comprehension is a function in CPython too, called where the
//!   expression is evaluated: its free variables are read while it runs,
//!   and nothing else runs meanwhile, so passing their current values as
//!   arguments is the same. A set comprehension builds the list and returns
//!   `set(...)` of it (the same insertions in the same order); a dict
//!   comprehension collects `(key, value)` pairs -- each key evaluated
//!   before its value, as CPython does -- and returns `dict(...)` of them.
//!   Module globals and builtins stay free names of the helper.
//! * A generator expression becomes a call of a generator helper
//!   (`genexpr`).
//! * A call of a module-level function with one starred argument
//!   (`f(a, *xs)`) becomes a call of a helper receiving the same arguments,
//!   which checks the starred value's length against the parameters it must
//!   fill -- raising CPython's own `TypeError` message when it does not --
//!   and calls `f` with its items.
//!
//! A comprehension is left alone in a class body, when it binds a name of
//! the enclosing scope (a walrus), or uses `super()`, `yield` or `await`;
//! a starred call when the callee is not a plain module-level function
//! with positional parameters only and no defaults.

mod genexpr;

use super::Fresh;
use super::INFER_MARKER;
use super::build::{
    assign, attribute, call, call_name, int, load, store, string, subscript,
};
use super::globals::bound_of;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    AtomicNodeIndex, CmpOp, Comprehension, ElifElseClause, Expr, ExprCompare, ExprContext,
    ExprList, Identifier, Parameter, ParameterWithDefault, Parameters, Stmt, StmtExpr,
    StmtFunctionDef, StmtIf, StmtRaise, StmtReturn,
};
use ruff_text_size::{Ranged, TextRange};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};

/// A module-level function a starred call may target: its positional
/// parameter names.
struct Callee {
    params: Vec<String>,
}

/// Rewrites `body` (a module), placing each helper right before the
/// top-level statement whose code uses it.
pub(crate) fn lift_helpers(body: &mut Vec<Stmt>, fresh: &Fresh) {
    let callees = module_callees(body);
    let module_names = bound_of(body).names;
    let generators = generator_names(body);
    let old = std::mem::take(body);
    for mut stmt in old {
        let lift = Lift {
            fresh,
            callees: &callees,
            module_names: &module_names,
            generators: &generators,
            consumer_arg: std::cell::Cell::new(false),
            out: RefCell::new(Vec::new()),
            scopes: RefCell::new(Vec::new()),
            lifted_inside: std::cell::Cell::new(0),
            force_next: RefCell::new(false),
        };
        lift.visit_stmt(&mut stmt);
        body.extend(lift.out.into_inner());
        body.push(stmt);
    }
}

/// Module-level `def`s bound once, with plain positional parameters.
fn module_callees(body: &[Stmt]) -> HashMap<String, Callee> {
    let mut defs: HashMap<String, Callee> = HashMap::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for stmt in body {
        if let Stmt::FunctionDef(def) = stmt {
            *seen.entry(def.name.to_string()).or_default() += 1;
            let p = &def.parameters;
            if def.decorator_list.is_empty()
                && !def.is_async
                && p.posonlyargs.is_empty()
                && p.kwonlyargs.is_empty()
                && p.vararg.is_none()
                && p.kwarg.is_none()
                && p.args.iter().all(|a| a.default.is_none())
            {
                defs.insert(
                    def.name.to_string(),
                    Callee {
                        params: p.args.iter().map(|a| a.parameter.name.to_string()).collect(),
                    },
                );
            }
        }
    }
    // A name bound more than once (two `def`s, or a `def` and an
    // assignment) is not a fixed callee.
    defs.retain(|name, _| {
        seen.get(name) == Some(&1) && super::closures::count_uses(body, name).bindings == 1
    });
    defs
}

#[derive(Clone)]
enum Frame {
    /// A function, lambda or comprehension scope, the names it binds, and
    /// those of them that keep one value once bound (`genexpr`).
    Local {
        names: HashSet<String>,
        stable: HashSet<String>,
    },
    /// A class body: nothing in it is rewritten.
    Class,
}

struct Lift<'a> {
    fresh: &'a Fresh,
    callees: &'a HashMap<String, Callee>,
    /// The names the module binds (shadowing a builtin).
    module_names: &'a HashSet<String>,
    /// The module's generator functions and generator method names.
    generators: &'a Generators,
    /// Set right before visiting a generator expression passed to a
    /// consuming builtin; read (and cleared) by `visit_expr`.
    consumer_arg: std::cell::Cell<bool>,
    out: RefCell<Vec<Stmt>>,
    scopes: RefCell<Vec<Frame>>,
    /// Helpers emitted so far inside the current comprehension body.
    lifted_inside: std::cell::Cell<usize>,
    /// Set by [`Lift::visit_comprehension`] for the comprehension it just
    /// walked, read right after by `visit_expr`.
    force_next: RefCell<bool>,
}

impl Lift<'_> {
    fn in_class(&self) -> bool {
        matches!(self.scopes.borrow().last(), Some(Frame::Class))
    }

    /// Whether `name` resolves to a local of some enclosing function,
    /// lambda or comprehension scope (rather than a global).
    fn is_local(&self, name: &str) -> bool {
        self.scopes
            .borrow()
            .iter()
            .any(|f| matches!(f, Frame::Local { names, .. } if names.contains(name)))
    }

    /// Whether the innermost scope binding `name` keeps one value for it.
    fn is_stable(&self, name: &str) -> bool {
        self.scopes.borrow().iter().rev().find_map(|f| match f {
            Frame::Local { names, stable } if names.contains(name) => Some(stable.contains(name)),
            _ => None,
        }) == Some(true)
    }

    /// Whether `name` is the builtin of that name here.
    fn builtin(&self, name: &str) -> bool {
        !self.is_local(name) && !self.module_names.contains(name)
    }

    fn with(&self, frame: Frame, f: impl FnOnce()) {
        self.scopes.borrow_mut().push(frame);
        f();
        self.scopes.borrow_mut().pop();
    }

    /// The names `expr` reads that resolve to an enclosing local, in a
    /// fixed order: they become the helper's parameters.
    fn free_locals(&self, expr: &Expr) -> Vec<String> {
        let mut reads = BTreeSet::new();
        free_names(expr, &mut reads);
        reads.into_iter().filter(|n| self.is_local(n)).collect()
    }

    fn helper_name(&self, kind: &str) -> String {
        format!("_pycc_{kind}{}", self.fresh.name().trim_start_matches("_pycc"))
    }

    fn emit(&self, name: &str, params: &[String], body: Vec<Stmt>, range: TextRange) {
        self.out.borrow_mut().push(Stmt::FunctionDef(StmtFunctionDef {
            node_index: AtomicNodeIndex::default(),
            range,
            is_async: false,
            decorator_list: Vec::new().into(),
            name: Identifier::new(name, range),
            type_params: None,
            parameters: Box::new(Parameters {
                range,
                node_index: AtomicNodeIndex::default(),
                posonlyargs: Vec::new().into(),
                args: params.iter().map(|p| param(p, range)).collect(),
                vararg: None,
                kwonlyargs: Vec::new().into(),
                kwarg: None,
            }),
            returns: Some(Box::new(load(INFER_MARKER, range))),
            body: body.into(),
        }));
    }

    /// `[elt for ...]` with several `for` clauses -> a helper call.
    /// `force`: lift even a single-`for` comprehension (one whose body
    /// calls a helper with its own variables, which only a helper's loop
    /// variables can type).
    fn comprehension(&self, expr: &Expr, force: bool) -> Option<Expr> {
        let (generators, kind) = match expr {
            Expr::ListComp(c) => (&c.generators, CompKind::List(&c.elt)),
            Expr::SetComp(c) => (&c.generators, CompKind::Set(&c.elt)),
            Expr::DictComp(c) => (&c.generators, CompKind::Dict(c.key.as_deref()?, &c.value)),
            _ => return None,
        };
        // A comprehension over a generator loops over it in a helper: only
        // a `for` statement iterates one.
        let force = force || generators.iter().any(|g| self.makes_generator(&g.iter));
        if (generators.len() < 2 && generators.iter().all(|g| g.ifs.len() < 2) && !force) || generators.iter().any(|g| g.is_async) || !liftable(expr) {
            return None;
        }
        let range = expr.range();
        let params = self.free_locals(expr);
        let result = self.fresh.name();
        let innermost = match kind {
            CompKind::List(elt) | CompKind::Set(elt) => vec![expr_stmt(
                call(
                    attribute(load(&result, range), "append", range),
                    vec![elt.clone()],
                    range,
                ),
                range,
            )],
            CompKind::Dict(key, value) => vec![expr_stmt(
                call(
                    attribute(load(&result, range), "append", range),
                    vec![super::build::tuple(
                        vec![key.clone(), value.clone()],
                        ExprContext::Load,
                        range,
                    )],
                    range,
                ),
                range,
            )],
        };
        let mut loops = innermost;
        for generator in generators.iter().rev() {
            loops = nest(generator, loops, range);
        }
        let empty = Expr::List(ExprList {
            node_index: AtomicNodeIndex::default(),
            range,
            elts: Vec::new(),
            ctx: ExprContext::Load,
        });
        let returned = match kind {
            CompKind::Set(_) => call_name("set", vec![load(&result, range)], range),
            CompKind::Dict(..) => call_name("dict", vec![load(&result, range)], range),
            CompKind::List(_) => load(&result, range),
        };
        let mut body = vec![assign(store(&result, range), empty, range)];
        body.extend(loops);
        body.push(Stmt::Return(StmtReturn {
            node_index: AtomicNodeIndex::default(),
            range,
            value: Some(Box::new(returned)),
        }));
        let name = self.helper_name("comp");
        self.emit(&name, &params, body, range);
        Some(call_name(
            &name,
            params.iter().map(|p| load(p, range)).collect(),
            range,
        ))
    }

    /// `f(a, *xs, b)` of a module-level `f` -> a helper call.
    fn starred_call(&self, expr: &Expr) -> Option<Expr> {
        let Expr::Call(c) = expr else { return None };
        let Expr::Name(func) = c.func.as_ref() else {
            return None;
        };
        if self.is_local(func.id.as_str()) || !c.arguments.keywords.is_empty() {
            return None;
        }
        let callee = self.callees.get(func.id.as_str())?;
        let args = &c.arguments.args;
        let stars: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| matches!(a, Expr::Starred(_)))
            .map(|(i, _)| i)
            .collect();
        let [star] = stars.as_slice() else {
            return None;
        };
        let star = *star;
        let n = callee.params.len();
        let fixed = args.len() - 1;
        let want = n.checked_sub(fixed)?;
        let range = c.range;
        let names: Vec<String> = (0..args.len()).map(|_| self.fresh.name()).collect();
        let seq = &names[star];
        let mut forwarded = Vec::new();
        for (i, name) in names.iter().enumerate() {
            if i == star {
                for k in 0..want {
                    forwarded.push(subscript(load(seq, range), int(k as u64, range), ExprContext::Load, range));
                }
            } else {
                forwarded.push(load(name, range));
            }
        }
        let fname = func.id.as_str();
        let length = self.fresh.name();
        let mut body = vec![assign(
            store(&length, range),
            call_name("len", vec![load(seq, range)], range),
            range,
        )];
        // Too few: one message per possible shortfall.
        for got in 0..want {
            let given = fixed + got;
            let missing: Vec<&String> = callee.params[given..].iter().collect();
            body.push(raise_if(
                compare(load(&length, range), CmpOp::Eq, int(got as u64, range), range),
                &missing_message(fname, &missing),
                range,
            ));
        }
        // Too many: the count is only known at run time.
        let too_many = format!(
            "{fname}() takes {n} positional argument{} but ",
            if n == 1 { "" } else { "s" }
        );
        body.push(Stmt::If(StmtIf {
            node_index: AtomicNodeIndex::default(),
            range,
            test: Box::new(compare(load(&length, range), CmpOp::Gt, int(want as u64, range), range)),
            body: vec![raise_type_error(
                binop_add(
                    binop_add(
                        string(&too_many, range),
                        call_name(
                            "str",
                            vec![binop_add(load(&length, range), int(fixed as u64, range), range)],
                            range,
                        ),
                        range,
                    ),
                    string(" were given", range),
                    range,
                ),
                range,
            )]
            .into(),
            elif_else_clauses: Vec::new(),
        }));
        body.push(Stmt::Return(StmtReturn {
            node_index: AtomicNodeIndex::default(),
            range,
            value: Some(Box::new(call_name(fname, forwarded, range))),
        }));
        let name = self.helper_name("star");
        self.emit(&name, &names, body, range);
        let call_args = args
            .iter()
            .map(|a| match a {
                Expr::Starred(s) => (*s.value).clone(),
                other => other.clone(),
            })
            .collect();
        Some(call_name(&name, call_args, range))
    }
}

enum CompKind<'a> {
    List(&'a Expr),
    Set(&'a Expr),
    Dict(&'a Expr, &'a Expr),
}

fn param(name: &str, range: TextRange) -> ParameterWithDefault {
    ParameterWithDefault {
        range,
        node_index: AtomicNodeIndex::default(),
        parameter: Parameter {
            range,
            node_index: AtomicNodeIndex::default(),
            name: Identifier::new(name, range),
            annotation: None,
        },
        default: None,
    }
}

fn expr_stmt(value: Expr, range: TextRange) -> Stmt {
    Stmt::Expr(StmtExpr {
        node_index: AtomicNodeIndex::default(),
        range,
        value: Box::new(value),
    })
}

fn compare(left: Expr, op: CmpOp, right: Expr, range: TextRange) -> Expr {
    Expr::Compare(ExprCompare {
        node_index: AtomicNodeIndex::default(),
        range,
        left: Box::new(left),
        ops: vec![op].into_boxed_slice(),
        comparators: vec![right].into_boxed_slice(),
    })
}

fn binop_add(left: Expr, right: Expr, range: TextRange) -> Expr {
    Expr::BinOp(ruff_python_ast::ExprBinOp {
        node_index: AtomicNodeIndex::default(),
        range,
        left: Box::new(left),
        op: ruff_python_ast::Operator::Add,
        right: Box::new(right),
    })
}

fn raise_type_error(message: Expr, range: TextRange) -> Stmt {
    Stmt::Raise(StmtRaise {
        node_index: AtomicNodeIndex::default(),
        range,
        exc: Some(Box::new(call_name("TypeError", vec![message], range))),
        cause: None,
    })
}

fn raise_if(test: Expr, message: &str, range: TextRange) -> Stmt {
    Stmt::If(StmtIf {
        node_index: AtomicNodeIndex::default(),
        range,
        test: Box::new(test),
        body: vec![raise_type_error(string(message, range), range)].into(),
        elif_else_clauses: Vec::<ElifElseClause>::new(),
    })
}

/// CPython's `f() missing 2 required positional arguments: 'b' and 'c'`.
fn missing_message(fname: &str, missing: &[&String]) -> String {
    let quoted: Vec<String> = missing.iter().map(|m| format!("'{m}'")).collect();
    let list = match quoted.len() {
        1 => quoted[0].clone(),
        2 => format!("{} and {}", quoted[0], quoted[1]),
        _ => format!(
            "{}, and {}",
            quoted[..quoted.len() - 1].join(", "),
            quoted[quoted.len() - 1]
        ),
    };
    format!(
        "{fname}() missing {} required positional argument{}: {list}",
        missing.len(),
        if missing.len() == 1 { "" } else { "s" }
    )
}

/// `body` nested in `generator`'s loop and its `if` clauses.
fn nest(generator: &Comprehension, body: Vec<Stmt>, range: TextRange) -> Vec<Stmt> {
    let mut inner = body;
    for cond in generator.ifs.iter().rev() {
        inner = vec![Stmt::If(StmtIf {
            node_index: AtomicNodeIndex::default(),
            range,
            test: Box::new(cond.clone()),
            body: inner.into(),
            elif_else_clauses: Vec::new(),
        })];
    }
    vec![super::build::for_loop(
        generator.target.clone(),
        generator.iter.clone(),
        inner,
        Vec::new(),
        range,
    )]
}

/// Whether a comprehension can become a helper: no walrus (it would bind in
/// the enclosing scope), no `super()` (it needs the method's class cell),
/// no `yield`/`await`.
fn liftable(expr: &Expr) -> bool {
    struct Check(bool);
    impl<'a> Visitor<'a> for Check {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Named(_) | Expr::Yield(_) | Expr::YieldFrom(_) | Expr::Await(_) => {
                    self.0 = false
                }
                Expr::Name(n) if n.id.as_str() == "super" => self.0 = false,
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut c = Check(true);
    c.visit_expr(expr);
    c.0
}

/// Every name `expr` reads that `expr` itself does not bind (comprehension
/// targets and lambda parameters are bound in their own scopes).
fn free_names(expr: &Expr, out: &mut BTreeSet<String>) {
    struct Reads<'o> {
        out: &'o mut BTreeSet<String>,
        bound: Vec<HashSet<String>>,
    }
    impl Reads<'_> {
        fn read(&mut self, name: &str) {
            if !self.bound.iter().any(|s| s.contains(name)) {
                self.out.insert(name.to_string());
            }
        }
        fn comprehension(&mut self, generators: &[Comprehension], elts: &[&Expr]) {
            let mut scope = HashSet::new();
            for (i, g) in generators.iter().enumerate() {
                if i == 0 {
                    self.visit_expr(&g.iter);
                }
                let mut names = HashSet::new();
                targets(&g.target, &mut names);
                scope.extend(names);
            }
            self.bound.push(scope);
            for (i, g) in generators.iter().enumerate() {
                if i > 0 {
                    self.visit_expr(&g.iter);
                }
                for cond in &g.ifs {
                    self.visit_expr(cond);
                }
            }
            for e in elts {
                self.visit_expr(e);
            }
            self.bound.pop();
        }
    }
    impl<'a> Visitor<'a> for Reads<'_> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Name(n) => self.read(n.id.as_str()),
                Expr::ListComp(c) => self.comprehension(&c.generators, &[&c.elt]),
                Expr::SetComp(c) => self.comprehension(&c.generators, &[&c.elt]),
                Expr::Generator(c) => self.comprehension(&c.generators, &[&c.elt]),
                Expr::DictComp(c) => match c.key.as_deref() {
                    Some(key) => self.comprehension(&c.generators, &[key, &c.value]),
                    None => self.comprehension(&c.generators, &[&c.value]),
                },
                Expr::Lambda(lambda) => {
                    let mut scope = HashSet::new();
                    if let Some(params) = lambda.parameters.as_deref() {
                        for p in params.iter() {
                            if let Some(d) = p.default() {
                                self.visit_expr(d);
                            }
                            scope.insert(p.name().to_string());
                        }
                    }
                    self.bound.push(scope);
                    self.visit_expr(&lambda.body);
                    self.bound.pop();
                }
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    Reads {
        out,
        bound: Vec::new(),
    }
    .visit_expr(expr);
}

fn targets(target: &Expr, out: &mut HashSet<String>) {
    match target {
        Expr::Name(n) => {
            out.insert(n.id.to_string());
        }
        Expr::Tuple(t) => t.elts.iter().for_each(|e| targets(e, out)),
        Expr::List(l) => l.elts.iter().for_each(|e| targets(e, out)),
        Expr::Starred(s) => targets(&s.value, out),
        _ => {}
    }
}

fn comprehension_scope(generators: &[Comprehension]) -> HashSet<String> {
    let mut names = HashSet::new();
    for g in generators {
        targets(&g.target, &mut names);
    }
    names
}

impl Transformer for Lift<'_> {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                for d in def.decorator_list.iter_mut() {
                    self.visit_expr(&mut d.expression);
                }
                let bound = bound_of(&def.body);
                let params: HashSet<String> =
                    def.parameters.iter().map(|p| p.name().to_string()).collect();
                let mut names = params.clone();
                names.extend(bound.names.iter().filter(|n| !bound.globals.contains(*n)).cloned());
                let stable = stable_names(&def.body, &names, &params);
                self.with(Frame::Local { names, stable }, || {
                    for s in def.body.iter_mut() {
                        self.visit_stmt(s);
                    }
                });
            }
            Stmt::ClassDef(class) => {
                self.with(Frame::Class, || {
                    for s in class.body.iter_mut() {
                        self.visit_stmt(s);
                    }
                });
            }
            _ => transformer::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&self, expr: &mut Expr) {
        let consumed = self.consumer_arg.replace(false);
        if self.in_class() {
            return;
        }
        // Children first, each in its own scope, so an inner comprehension
        // is already a call when the outer one is lifted.
        match expr {
            Expr::ListComp(c) => {
                let scope = comprehension_scope(&c.generators);
                self.visit_comprehension(&mut c.generators, scope, &mut [c.elt.as_mut()]);
            }
            Expr::SetComp(c) => {
                let scope = comprehension_scope(&c.generators);
                self.visit_comprehension(&mut c.generators, scope, &mut [c.elt.as_mut()]);
            }
            Expr::Generator(c) => {
                let scope = comprehension_scope(&c.generators);
                self.visit_comprehension(&mut c.generators, scope, &mut [c.elt.as_mut()]);
            }
            Expr::DictComp(c) => {
                let scope = comprehension_scope(&c.generators);
                match c.key.as_deref_mut() {
                    Some(key) => self.visit_comprehension(
                        &mut c.generators,
                        scope,
                        &mut [key, c.value.as_mut()],
                    ),
                    None => self.visit_comprehension(&mut c.generators, scope, &mut [c.value.as_mut()]),
                }
            }
            Expr::Lambda(lambda) => {
                let mut names = HashSet::new();
                if let Some(params) = lambda.parameters.as_deref() {
                    for p in params.iter() {
                        names.insert(p.name().to_string());
                    }
                }
                let stable = names.clone();
                self.with(Frame::Local { names, stable }, || self.visit_expr(&mut lambda.body));
            }
            Expr::Call(c) => {
                self.materialize_argument(c);
                let consumer = match c.func.as_ref() {
                    Expr::Name(f) => genexpr::is_consumer(f.id.as_str()) && self.builtin(f.id.as_str()),
                    Expr::Attribute(a) => a.attr.as_str() == "join",
                    _ => false,
                };
                self.visit_expr(&mut c.func);
                for arg in c.arguments.args.iter_mut() {
                    self.consumer_arg.set(consumer && matches!(arg, Expr::Generator(_)));
                    self.visit_expr(arg);
                }
                for keyword in c.arguments.keywords.iter_mut() {
                    self.visit_expr(&mut keyword.value);
                }
            }
            _ => transformer::walk_expr(self, expr),
        }
        let force = std::mem::replace(&mut *self.force_next.borrow_mut(), false);
        if let Some(replacement) = self
            .comprehension(expr, force)
            .or_else(|| self.generator_expression(expr, consumed))
            .or_else(|| self.starred_call(expr))
        {
            *expr = replacement;
            self.lifted_inside.set(self.lifted_inside.get() + 1);
        }
    }
}

/// The generator functions (module-level `def`s containing `yield`) and
/// the method names every class defining them makes generators.
pub(super) struct Generators {
    functions: HashSet<String>,
    methods: HashSet<String>,
}

fn generator_names(body: &[Stmt]) -> Generators {
    let mut functions = HashSet::new();
    let mut methods = HashSet::new();
    let mut plain_methods = HashSet::new();
    for stmt in body {
        match stmt {
            Stmt::FunctionDef(def) if super::ext_generators::yields(&def.body) => {
                functions.insert(def.name.to_string());
            }
            Stmt::ClassDef(class) => {
                for item in class.body.iter() {
                    if let Stmt::FunctionDef(def) = item {
                        if super::ext_generators::yields(&def.body) {
                            methods.insert(def.name.to_string());
                        } else {
                            plain_methods.insert(def.name.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    methods.retain(|m| !plain_methods.contains(m));
    Generators { functions, methods }
}

impl Lift<'_> {
    /// Whether `iter` is a call making a generator: of a module-level
    /// generator function, a generator method, or a lifted generator
    /// expression.
    fn makes_generator(&self, iter: &Expr) -> bool {
        let Expr::Call(c) = iter else { return false };
        match c.func.as_ref() {
            Expr::Name(f) => {
                !self.is_local(f.id.as_str())
                    && (self.generators.functions.contains(f.id.as_str())
                        || f.id.as_str().starts_with("_pycc_genexpr_"))
            }
            Expr::Attribute(a) => self.generators.methods.contains(a.attr.as_str()),
            _ => false,
        }
    }
}

/// The names of `names` a function body binds at most once, before any
/// read: a parameter never rebound, or a variable assigned by one top-level
/// statement and nowhere else.
fn stable_names(body: &[Stmt], names: &HashSet<String>, params: &HashSet<String>) -> HashSet<String> {
    // A name target, or each name of a flat tuple target (`pos, key = ...`).
    let names_of = |target: &Expr| -> Vec<String> {
        match target {
            Expr::Name(n) => vec![n.id.to_string()],
            Expr::Tuple(t) if t.elts.iter().all(|e| matches!(e, Expr::Name(_))) => t
                .elts
                .iter()
                .filter_map(|e| match e {
                    Expr::Name(n) => Some(n.id.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    };
    let top_level: HashSet<String> = body
        .iter()
        .flat_map(|s| match s {
            Stmt::Assign(a) if a.targets.len() == 1 => names_of(&a.targets[0]),
            Stmt::AnnAssign(a) if a.value.is_some() => match a.target.as_ref() {
                Expr::Name(n) => vec![n.id.to_string()],
                _ => Vec::new(),
            },
            _ => Vec::new(),
        })
        .collect();
    names
        .iter()
        .filter(|n| {
            let bindings = super::closures::count_uses(body, n).bindings;
            if params.contains(*n) {
                bindings == 0
            } else {
                bindings == 1 && top_level.contains(*n)
            }
        })
        .cloned()
        .collect()
}

impl Lift<'_> {
    /// A generator expression passed as the only argument of a consumer
    /// that materializes it becomes the list comprehension of the same
    /// elements (`comps::materialize_generator`), lifted like any other.
    fn materialize_argument(&self, c: &mut ruff_python_ast::ExprCall) {
        if c.arguments.args.len() != 1 || !c.arguments.keywords.is_empty() {
            return;
        }
        let consumer = match c.func.as_ref() {
            Expr::Name(f)
                if super::comps::is_materializing_consumer(f.id.as_str())
                    && self.builtin(f.id.as_str()) =>
            {
                f.id.to_string()
            }
            Expr::Attribute(a) if a.attr.as_str() == "join" => "join".to_string(),
            _ => return,
        };
        if let Some(list) = super::comps::materialize_generator(&consumer, &c.arguments.args[0]) {
            c.arguments.args[0] = list;
        }
    }

    fn visit_comprehension(
        &self,
        generators: &mut [Comprehension],
        scope: HashSet<String>,
        elts: &mut [&mut Expr],
    ) {
        if let Some(first) = generators.first_mut() {
            self.visit_expr(&mut first.iter);
        }
        let before = self.lifted_inside.get();
        self.with(Frame::Local { names: scope, stable: HashSet::new() }, || {
            for (i, g) in generators.iter_mut().enumerate() {
                if i > 0 {
                    self.visit_expr(&mut g.iter);
                }
                for cond in g.ifs.iter_mut() {
                    self.visit_expr(cond);
                }
            }
            for e in elts.iter_mut() {
                self.visit_expr(e);
            }
        });
        if self.lifted_inside.get() != before {
            *self.force_next.borrow_mut() = true;
        }
    }
}
