//! Calls bound to the full Python signature: `*args`, keyword-only
//! parameters, `**kwargs`, defaults of any kind, and calls with keywords,
//! `*seq` and `**mapping`.
//!
//! A `def` whose signature HIR cannot represent directly -- one with a
//! `*args`, a keyword-only parameter, a `**kwargs`, or a default that is not
//! a literal -- is *flattened*: every parameter becomes a plain positional
//! one, in order (positional-only, positional-or-keyword, `*args` as a
//! `list[T]`, keyword-only, `**kwargs` as a `dict[str, T]`), with no
//! defaults. A default that is not a literal is evaluated once, where the
//! `def` is, into a module global (`_pycc_default_<f>_<param>`), so a
//! mutable default is shared between calls as in CPython. The body first
//! marks its `*args` list a tuple ([`MARK_TUPLE`]: it prints, compares and
//! hashes as one).
//!
//! Every call of a flattened function, and every call of any module
//! function with `**mapping`, is bound here exactly as CPython binds it
//! (`ceval.c`'s `initialize_locals`), with CPython's `TypeError` messages:
//!
//! * a call whose arguments are all known statically becomes a call with
//!   the flat argument list -- extra positionals in a list display, extra
//!   keywords in a dict display, omitted parameters filled from their
//!   defaults -- written in place when that keeps the arguments' evaluation
//!   order, else through a helper receiving them in source order;
//! * a call that cannot bind (a missing, unexpected or doubled argument)
//!   becomes a helper that evaluates every argument and raises the
//!   `TypeError`;
//! * a call with `*seq` or `**mapping` becomes a helper binding at run
//!   time, checking in CPython's order ([`emit`]). A `**` operand that is a
//!   dict display of identifier keys is spliced into keyword arguments
//!   first, and the call is then bound here whatever its callee
//!   ([`splice`]), so one that cannot bind still raises at run time.
//!
//! Methods of top-level classes are bound the same way when one class
//! alone defines their name (`obj.m(...)`, `C.m(...)` of a static method),
//! and so is a class's constructor (`C(...)`). A flattened function used as
//! a value is called with its flat parameters (`docs/DEVIATIONS.md`).

mod emit;
mod splice;

use super::Fresh;
use super::build::load;
use emit::{Emitter, Receiver};
pub(super) use emit::parse_helper;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{
    AtomicNodeIndex, Expr, ExprContext, ExprSubscript, Parameter, ParameterWithDefault, Parameters,
    Stmt, StmtFunctionDef,
};
use ruff_text_size::{Ranged, TextRange};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// The builtin marking a list as a tuple (`pycc_hir` lowers it).
pub const MARK_TUPLE: &str = "__pycc_mark_tuple";

/// One parameter of a signature.
#[derive(Clone)]
pub(super) struct Param {
    pub(super) name: String,
    /// The default, as an expression valid at any call site of the
    /// module: a literal, or a read of the module global it was hoisted
    /// into.
    pub(super) default: Option<Expr>,
}

/// A function's or method's signature.
#[derive(Clone)]
pub(super) struct Sig {
    /// `f`, or `C.m` for a method: the name CPython's messages use.
    pub(super) qualname: String,
    /// Positional-only parameters (a method's receiver first).
    pub(super) posonly: Vec<Param>,
    /// Positional-or-keyword parameters.
    pub(super) regular: Vec<Param>,
    pub(super) vararg: Option<String>,
    pub(super) kwonly: Vec<Param>,
    pub(super) kwarg: Option<String>,
    /// Whether the definition was flattened (else it is called as
    /// written, with its own positional parameters).
    pub(super) flat: bool,
    /// The return annotation, for a helper that only raises.
    pub(super) returns: Option<Expr>,
}

impl Sig {
    /// Every positional parameter, positional-only first.
    pub(super) fn positional(&self) -> impl Iterator<Item = &Param> {
        self.posonly.iter().chain(&self.regular)
    }

    pub(super) fn positional_count(&self) -> usize {
        self.posonly.len() + self.regular.len()
    }
}

/// Whether `default` is a literal HIR materializes itself.
pub(super) fn is_literal(default: &Expr) -> bool {
    match default {
        Expr::NumberLiteral(_)
        | Expr::StringLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_) => true,
        Expr::UnaryOp(u) => matches!(*u.operand, Expr::NumberLiteral(_)),
        _ => false,
    }
}

/// Whether HIR cannot represent `parameters` as written.
fn needs_flattening(parameters: &Parameters) -> bool {
    parameters.vararg.is_some()
        || !parameters.kwonlyargs.is_empty()
        || parameters.kwarg.is_some()
        || parameters
            .posonlyargs
            .iter()
            .chain(&parameters.args)
            .chain(&parameters.kwonlyargs)
            .any(|p| p.default.as_deref().is_some_and(|d| !is_literal(d)))
}

/// The factory of a `default_factory` default
/// (`super::dataclasses::FACTORY_MARKER`), if `default` is one.
fn factory_default(default: &Expr) -> Option<&Expr> {
    let Expr::Call(call) = default else {
        return None;
    };
    match (&*call.func, call.arguments.args.as_ref()) {
        (Expr::Name(n), [factory]) if n.id.as_str() == super::dataclasses::FACTORY_MARKER => {
            Some(factory)
        }
        _ => None,
    }
}

/// The global a hoisted default lives in.
pub(super) fn default_global(qualname: &str, param: &str) -> String {
    format!("_pycc_default_{}_{param}", qualname.replace('.', "_"))
}

/// Reads `def`'s signature; when `flatten`, rewrites the `def` (see the
/// module doc), pushing the assignments hoisting its defaults.
fn take_signature(
    def: &mut StmtFunctionDef,
    qualname: String,
    flatten: bool,
    hoisted: &mut Vec<Stmt>,
) -> Sig {
    let range = def.range;
    let mut param = |p: &mut ParameterWithDefault| -> Param {
        let name = p.parameter.name.to_string();
        let annotation = p.parameter.annotation.as_deref().cloned();
        let default = p.default.as_deref().map(|d| {
            // A dataclass `default_factory` (`super::dataclasses`): the
            // factory is called at every call site omitting the argument.
            if let Some(factory) = factory_default(d) {
                return super::build::call(factory.clone(), Vec::new(), range);
            }
            if is_literal(d) || !flatten {
                d.clone()
            } else {
                // Annotated with the parameter's own type, so an empty
                // display (`a: list[int] = []`) has an element type.
                let global = default_global(&qualname, &name);
                let target = super::build::store(&global, range);
                hoisted.push(match annotation.clone() {
                    Some(annotation) => {
                        super::build::ann_assign(target, annotation, d.clone(), range)
                    }
                    None => super::build::assign(target, d.clone(), range),
                });
                load(&global, range)
            }
        });
        Param { name, default }
    };
    let parameters = def.parameters.as_mut();
    let posonly: Vec<Param> = parameters.posonlyargs.iter_mut().map(&mut param).collect();
    let regular: Vec<Param> = parameters.args.iter_mut().map(&mut param).collect();
    let kwonly: Vec<Param> = parameters.kwonlyargs.iter_mut().map(&mut param).collect();
    let sig = Sig {
        qualname,
        posonly,
        regular,
        vararg: parameters.vararg.as_ref().map(|v| v.name.to_string()),
        kwonly,
        kwarg: parameters.kwarg.as_ref().map(|v| v.name.to_string()),
        flat: flatten,
        returns: def.returns.as_deref().cloned(),
    };
    if flatten {
        flatten_parameters(def);
    }
    sig
}

/// `container[T]` (`dict[str, T]`) for a flattened `*args`/`**kwargs`
/// annotation `T`.
fn wrapped(container: &str, annotation: Option<&Expr>, range: TextRange) -> Option<Box<Expr>> {
    let inner = annotation?.clone();
    let slice = if container == "dict" {
        super::build::tuple(vec![load("str", range), inner], ExprContext::Load, range)
    } else {
        inner
    };
    Some(Box::new(Expr::Subscript(ExprSubscript {
        node_index: AtomicNodeIndex::default(),
        range,
        value: Box::new(load(container, range)),
        slice: Box::new(slice),
        ctx: ExprContext::Load,
    })))
}

/// Rewrites `def`'s parameters to the flat positional list, without
/// defaults, and marks its `*args` list a tuple.
fn flatten_parameters(def: &mut StmtFunctionDef) {
    let range = def.range;
    let parameters = def.parameters.as_mut();
    let plain = |parameter: Parameter| ParameterWithDefault {
        range,
        node_index: AtomicNodeIndex::default(),
        parameter,
        default: None,
    };
    let mut flat: Vec<ParameterWithDefault> = Vec::new();
    flat.extend(std::mem::take(&mut parameters.posonlyargs));
    flat.extend(std::mem::take(&mut parameters.args));
    let vararg = parameters.vararg.take();
    if let Some(vararg) = &vararg {
        let mut p = (**vararg).clone();
        p.annotation = wrapped("list", vararg.annotation.as_deref(), range);
        flat.push(plain(p));
    }
    flat.extend(std::mem::take(&mut parameters.kwonlyargs));
    if let Some(kwarg) = parameters.kwarg.take() {
        let mut p = (*kwarg).clone();
        p.annotation = wrapped("dict", kwarg.annotation.as_deref(), range);
        flat.push(plain(p));
    }
    for p in &mut flat {
        p.default = None;
    }
    parameters.args = flat.into();
    if let Some(vararg) = vararg {
        let mark = Stmt::Expr(ruff_python_ast::StmtExpr {
            node_index: AtomicNodeIndex::default(),
            range,
            value: Box::new(super::build::call_name(
                MARK_TUPLE,
                vec![load(vararg.name.as_str(), range)],
                range,
            )),
        });
        let at = usize::from(matches!(
            def.body.first(),
            Some(Stmt::Expr(e)) if matches!(*e.value, Expr::StringLiteral(_))
        ));
        def.body.insert(at, mark);
    }
}

/// The module's bindable signatures.
#[derive(Default)]
struct Sigs {
    /// Module functions by name.
    functions: HashMap<String, Sig>,
    /// Class constructors (`__init__`, receiver included) by class name.
    constructors: HashMap<String, Sig>,
    /// Methods by name, when exactly one class defines that name: the
    /// signature (receiver included unless static), whether it is static,
    /// and its class.
    methods: HashMap<String, (Sig, bool, String)>,
    /// Top-level class names.
    classes: HashSet<String>,
    /// Names a module-level `import` binds (`import builtins as b`,
    /// `from os import path`): a method call on one is a call on a foreign
    /// object, never on an instance of the module's class that defines a
    /// method of the same name.
    imported: HashSet<String>,
}

impl Sigs {
    fn any_flat(&self) -> bool {
        self.functions.values().any(|s| s.flat)
            || self
                .constructors
                .values()
                .any(|s| s.flat || s.positional().any(|p| p.default.is_some()))
            || self.methods.values().any(|(s, ..)| s.flat)
    }
}

/// The `staticmethod`-only decorator list check: `Some(true)` for a
/// static method, `Some(false)` for a plain one, `None` for any other.
fn method_kind(def: &StmtFunctionDef) -> Option<bool> {
    match def.decorator_list.as_slice() {
        [] => Some(false),
        [d] if matches!(&d.expression, Expr::Name(n) if n.id.as_str() == "staticmethod") => {
            Some(true)
        }
        _ => None,
    }
}

/// The names the module-level `import` statements of `body` bind, those
/// under a module-level `if` or `try` included.
fn imported_names(body: &[Stmt], out: &mut HashSet<String>) {
    for stmt in body {
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    let name = match &alias.asname {
                        Some(asname) => asname.as_str(),
                        None => alias.name.as_str().split('.').next().unwrap_or_default(),
                    };
                    out.insert(name.to_string());
                }
            }
            Stmt::ImportFrom(import) => {
                for alias in &import.names {
                    out.insert(alias.asname.as_ref().unwrap_or(&alias.name).to_string());
                }
            }
            Stmt::If(s) => {
                imported_names(&s.body, out);
                for clause in &s.elif_else_clauses {
                    imported_names(&clause.body, out);
                }
            }
            Stmt::Try(s) => {
                imported_names(&s.body, out);
                for handler in &s.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    imported_names(&h.body, out);
                }
                imported_names(&s.orelse, out);
                imported_names(&s.finalbody, out);
            }
            _ => {}
        }
    }
}

/// Collects (and flattens) every bindable `def` of the module.
fn collect(body: &mut Vec<Stmt>) -> Sigs {
    let mut sigs = Sigs::default();
    let bound_once: HashSet<String> = body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::FunctionDef(def) => Some(def.name.to_string()),
            _ => None,
        })
        .filter(|name| super::closures::count_uses(body, name).bindings == 1)
        .collect();
    imported_names(body, &mut sigs.imported);
    let mut method_counts: HashMap<String, usize> = HashMap::new();
    for stmt in body.iter() {
        if let Stmt::ClassDef(class) = stmt {
            sigs.classes.insert(class.name.to_string());
            for member in class.body.iter() {
                if let Stmt::FunctionDef(def) = member {
                    *method_counts.entry(def.name.to_string()).or_default() += 1;
                }
            }
        }
    }
    let old = std::mem::take(body);
    for mut stmt in old {
        let mut hoisted = Vec::new();
        match &mut stmt {
            Stmt::FunctionDef(def)
                if !def.is_async
                    && def.decorator_list.is_empty()
                    && bound_once.contains(def.name.as_str()) =>
            {
                let flatten = needs_flattening(&def.parameters);
                let name = def.name.to_string();
                let sig = take_signature(def, name.clone(), flatten, &mut hoisted);
                sigs.functions.insert(name, sig);
            }
            Stmt::ClassDef(class) if class.decorator_list.is_empty() => {
                let class_name = class.name.to_string();
                for member in class.body.iter_mut() {
                    let Stmt::FunctionDef(def) = member else {
                        continue;
                    };
                    let Some(is_static) = method_kind(def) else {
                        continue;
                    };
                    if def.is_async {
                        continue;
                    }
                    let flatten = needs_flattening(&def.parameters);
                    let qualname = format!("{class_name}.{}", def.name);
                    let sig = take_signature(def, qualname, flatten, &mut hoisted);
                    if def.name.as_str() == "__init__" && !is_static {
                        let mut ctor = sig.clone();
                        ctor.returns = Some(load(&class_name, def.range));
                        sigs.constructors.insert(class_name.clone(), ctor);
                    } else if method_counts.get(def.name.as_str()) == Some(&1) {
                        sigs.methods
                            .insert(def.name.to_string(), (sig, is_static, class_name.clone()));
                    }
                }
                // A class without its own `__init__` is constructed through
                // the first base's (in base order) that has one.
                if !sigs.constructors.contains_key(&class_name) {
                    let inherited = class.arguments.as_ref().and_then(|args| {
                        args.args.iter().find_map(|base| match base {
                            Expr::Name(n) => sigs.constructors.get(n.id.as_str()).cloned(),
                            _ => None,
                        })
                    });
                    if let Some(mut ctor) = inherited {
                        ctor.returns = Some(load(&class_name, class.range));
                        sigs.constructors.insert(class_name.clone(), ctor);
                    }
                }
            }
            _ => {}
        }
        body.extend(hoisted);
        body.push(stmt);
    }
    sigs
}

/// One argument of a call, in source order.
#[derive(Clone)]
pub(super) enum Arg {
    Positional(Expr),
    Star(Expr),
    Keyword(String, Expr),
    DoubleStar(Expr),
}

impl Arg {
    pub(super) fn value(&self) -> &Expr {
        match self {
            Arg::Positional(e) | Arg::Star(e) | Arg::Keyword(_, e) | Arg::DoubleStar(e) => e,
        }
    }
}

/// The arguments of a call, in source order.
fn call_args(arguments: &ruff_python_ast::Arguments) -> Vec<Arg> {
    let mut items: Vec<(TextRange, Arg)> = Vec::new();
    for arg in arguments.args.iter() {
        let item = match arg {
            Expr::Starred(s) => Arg::Star((*s.value).clone()),
            other => Arg::Positional(other.clone()),
        };
        items.push((arg.range(), item));
    }
    for kw in arguments.keywords.iter() {
        let item = match &kw.arg {
            Some(name) => Arg::Keyword(name.to_string(), kw.value.clone()),
            None => Arg::DoubleStar(kw.value.clone()),
        };
        items.push((kw.range, item));
    }
    items.sort_by_key(|(range, _)| range.start());
    items.into_iter().map(|(_, a)| a).collect()
}

/// The call rewriter: the scopes of the function bodies it is inside (the
/// names each binds), and the helpers it emitted.
struct Rewrite<'a> {
    sigs: &'a Sigs,
    fresh: &'a Fresh,
    scopes: RefCell<Vec<HashSet<String>>>,
    out: RefCell<Vec<Stmt>>,
}

impl Rewrite<'_> {
    fn shadowed(&self, name: &str) -> bool {
        self.scopes.borrow().iter().any(|s| s.contains(name))
    }

    /// The signature, callee name and receiver of a call of `func`.
    fn callee(&self, func: &Expr) -> Option<(&Sig, String, Receiver)> {
        match func {
            Expr::Name(n) if !self.shadowed(n.id.as_str()) => {
                let name = n.id.to_string();
                if let Some(sig) = self.sigs.functions.get(&name) {
                    return Some((sig, name, Receiver::None));
                }
                let sig = self.sigs.constructors.get(&name)?;
                Some((sig, name, Receiver::Implicit))
            }
            Expr::Attribute(a) => {
                let (sig, is_static, class) = self.sigs.methods.get(a.attr.as_str())?;
                let attr = a.attr.to_string();
                match a.value.as_ref() {
                    // `module.m(...)`: a foreign call (`Sigs::imported`).
                    Expr::Name(n)
                        if self.sigs.imported.contains(n.id.as_str())
                            && !self.shadowed(n.id.as_str()) =>
                    {
                        None
                    }
                    Expr::Name(n)
                        if self.sigs.classes.contains(n.id.as_str())
                            && !self.shadowed(n.id.as_str()) =>
                    {
                        // `C.m(...)`: only a static method of that class.
                        (*is_static && n.id.as_str() == class)
                            .then(|| (sig, attr, Receiver::Class(class.clone())))
                    }
                    receiver if *is_static => {
                        Some((sig, attr, Receiver::Unbound(receiver.clone())))
                    }
                    receiver => Some((sig, attr, Receiver::Bound(receiver.clone()))),
                }
            }
            _ => None,
        }
    }

    /// The rewrite of `call`, or `None` to leave it as written.
    fn rewrite_call(&self, call_expr: &ruff_python_ast::ExprCall) -> Option<Expr> {
        let (sig, callee, receiver) = self.callee(&call_expr.func)?;
        let spliced_call = splice::spliced(call_expr);
        let call_expr = spliced_call.as_ref().unwrap_or(call_expr);
        let args = call_args(&call_expr.arguments);
        let mapping = args.iter().any(|a| matches!(a, Arg::DoubleStar(_)));
        let star = args.iter().any(|a| matches!(a, Arg::Star(_)));
        // An unflattened function keeps HIR's keyword binding and literal
        // defaults; only a call with `**mapping`, or with `*seq` when a
        // default may fill a slot the sequence leaves empty, is bound here.
        // A `*seq` call of a function without defaults keeps the starred
        // unpacking `desugar::starred` gives it, which takes a tuple too.
        let defaults = sig.positional().any(|p| p.default.is_some());
        // HIR fills omitted defaults for function and method calls but not
        // for a constructor call, so one of a constructor with defaults is
        // bound here.
        let constructor = matches!(receiver, Receiver::Implicit) && defaults;
        // HIR binds no keyword of a method call: `self.m(k=v)` is bound here
        // when every keyword names a parameter of `m` (a call that cannot
        // bind keeps CPython's dispatch, in case `self` is not the class's).
        let self_keywords = matches!(&receiver, Receiver::Bound(Expr::Name(n)) if n.id.as_str() == "self")
            && args.iter().any(|a| matches!(a, Arg::Keyword(..)))
            && !star
            && args.iter().all(|a| match a {
                Arg::Keyword(name, _) => sig
                    .positional()
                    .chain(&sig.kwonly)
                    .any(|p| p.name == *name)
                    || sig.kwarg.is_some(),
                _ => true,
            });
        if !sig.flat
            && !mapping
            && (!star || !defaults)
            && !constructor
            && !self_keywords
            && spliced_call.is_none()
        {
            return None;
        }
        let emitter = Emitter {
            fresh: self.fresh,
            sig,
            range: call_expr.range,
            callee,
            receiver,
            returns: sig.returns.clone(),
            module_qualname: format!("__main__.{}", sig.qualname),
        };
        let (expr, helpers) = if mapping || star {
            emitter.dynamic(&args)
        } else {
            emitter.fixed(&args)
        };
        self.out.borrow_mut().extend(helpers);
        Some(expr)
    }
}

impl Transformer for Rewrite<'_> {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        if let Stmt::FunctionDef(def) = stmt {
            let mut names = super::stored_names(&def.body);
            let p = &def.parameters;
            for param in p.posonlyargs.iter().chain(&p.args).chain(&p.kwonlyargs) {
                names.insert(param.parameter.name.to_string());
            }
            for v in p.vararg.iter().chain(&p.kwarg) {
                names.insert(v.name.to_string());
            }
            // A name the function declares `global` is the module's.
            for s in def.body.iter() {
                if let Stmt::Global(g) = s {
                    for n in &g.names {
                        names.remove(n.as_str());
                    }
                }
            }
            self.scopes.borrow_mut().push(names);
            transformer::walk_stmt(self, stmt);
            self.scopes.borrow_mut().pop();
            return;
        }
        transformer::walk_stmt(self, stmt);
    }

    fn visit_expr(&self, expr: &mut Expr) {
        transformer::walk_expr(self, expr);
        if let Expr::Call(c) = expr
            && let Some(rewritten) = self.rewrite_call(c)
        {
            *expr = rewritten;
        }
    }
}

/// Whether some call of the module passes `*seq` or `**mapping`, or is a
/// `self.m(...)` call with keywords.
fn has_mapping_call(body: &[Stmt]) -> bool {
    use ruff_python_ast::visitor::{self, Visitor};
    struct Find(bool);
    impl<'a> Visitor<'a> for Find {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Call(c) = expr
                && (c.arguments.keywords.iter().any(|k| k.arg.is_none())
                    || c.arguments
                        .args
                        .iter()
                        .any(|a| matches!(a, Expr::Starred(_)))
                    || (!c.arguments.keywords.is_empty()
                        && matches!(c.func.as_ref(), Expr::Attribute(a)
                            if matches!(a.value.as_ref(), Expr::Name(n) if n.id.as_str() == "self"))))
            {
                self.0 = true;
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut find = Find(false);
    for stmt in body {
        find.visit_stmt(stmt);
    }
    find.0
}

/// Rewrites `body` (a module): see the module documentation.
pub(crate) fn bind_signatures(body: &mut Vec<Stmt>, fresh: &Fresh) {
    let sigs = collect(body);
    if !sigs.any_flat() && !has_mapping_call(body) {
        return;
    }
    let old = std::mem::take(body);
    for mut stmt in old {
        let rewrite = Rewrite {
            sigs: &sigs,
            fresh,
            scopes: RefCell::new(Vec::new()),
            out: RefCell::new(Vec::new()),
        };
        rewrite.visit_stmt(&mut stmt);
        body.extend(rewrite.out.into_inner());
        body.push(stmt);
    }
}
