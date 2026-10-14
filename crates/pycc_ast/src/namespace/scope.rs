//! Python's name resolution, applied to the references of a module that
//! resolve to its own global scope ([`rewrite_module_globals`]), and the
//! scan of what a module's global scope binds ([`module_bindings`]).
//!
//! The rules are CPython's symbol-table rules: a name a function binds
//! (a parameter, an assignment target, a `def`/`class`/`import` name, an
//! `except ... as` or pattern capture) is local to it unless declared
//! `global` (then it is the module's) or `nonlocal` (an enclosing
//! function's); a class body's bindings are visible only in the class body
//! itself, never in its methods; a comprehension's targets are local to
//! it, its first iterable is evaluated in the enclosing scope, and an
//! assignment expression inside it binds the enclosing function's (or the
//! module's) name; a lambda is a function. Decorators, default values,
//! annotations and class bases are evaluated in the enclosing scope.
//! `import` statements are left alone: the driver replaces a module's
//! project imports itself.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ops::Range;

use ruff_python_ast::name::Name;
use ruff_python_ast::visitor::transformer::{self as tr, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    AtomicNodeIndex, Comprehension, ExceptHandler, Expr, ExprContext, ExprName, Identifier,
    ModModule, Parameters, Pattern, Stmt, TypeParam, TypeParams,
};
use ruff_text_size::{Ranged, TextRange};

/// What a module-scope name use becomes.
pub enum NameRewrite {
    /// Left as written.
    Keep,
    /// Spelled with the given name instead.
    Rename(String),
    /// Refused with a diagnostic (`code`, `message`) at the use.
    Refuse { code: &'static str, message: String },
}

/// What a module-scope attribute chain `root.a.b...` becomes.
pub enum ChainRewrite {
    /// The root and its first `consumed` attributes become the name `name`
    /// (or, with `cell`, the element `name[0]` of the one-element list a
    /// module global some function rebinds lives in); the remaining
    /// attributes are read from it as before.
    Name {
        consumed: usize,
        name: String,
        cell: bool,
    },
    /// Refused with a diagnostic at the whole chain.
    Refuse { code: &'static str, message: String },
}

/// The decisions [`rewrite_module_globals`] applies.
pub trait GlobalRewrite {
    /// A use of module-scope `name`. `ctx` is `Load`, `Store` or `Del` for
    /// an expression or a binding identifier, and `Invalid` for the name of
    /// a `global` declaration (a declaration is never refused).
    fn name(&self, name: &str, ctx: ExprContext) -> NameRewrite;
    /// An attribute chain `root.attrs[0].attrs[1]...` whose root resolves to
    /// the module scope; `ctx` is the outermost attribute's. `None` leaves
    /// the chain alone (its root is then visited as a plain name).
    fn chain(&self, root: &str, attrs: &[&str], ctx: ExprContext) -> Option<ChainRewrite>;
}

/// A refusal reported by a [`GlobalRewrite`], at a source range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteError {
    pub code: &'static str,
    pub message: String,
    pub range: Range<u32>,
}

/// What a module's global scope binds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ModuleBindings {
    /// Every name the module scope binds other than by an `import`: its
    /// top-level `def`/`class` names, assignment and loop targets, `except
    /// ... as`/pattern captures, assignment expressions, and every name a
    /// function declares `global`.
    pub defined: BTreeSet<String>,
    /// Every name an `import` statement in the module scope binds.
    pub imported: BTreeSet<String>,
    /// Whether the module mentions `__name__` anywhere.
    pub mentions_dunder_name: bool,
}

/// Scans what `module`'s global scope binds. A name bound both by an
/// `import` and otherwise is reported only as `imported`.
pub fn module_bindings(module: &ModModule) -> ModuleBindings {
    let scope = collect_scope(&module.body);
    let mut globals = GlobalDecls::default();
    globals.visit_body(&module.body);
    let mut defined: BTreeSet<String> = scope.bound.into_iter().chain(globals.names).collect();
    defined.retain(|name| !scope.imported.contains(name));
    ModuleBindings {
        defined,
        imported: scope.imported,
        mentions_dunder_name: globals.dunder_name,
    }
}

/// Rewrites every reference in `module` that resolves to its global scope
/// as `rewrite` decides, returning the refusals in source order.
pub fn rewrite_module_globals(
    module: &mut ModModule,
    rewrite: &dyn GlobalRewrite,
) -> Vec<RewriteError> {
    let module_scope = collect_scope(&module.body);
    let rewriter = Rewriter {
        rewrite,
        scopes: RefCell::new(vec![Scope {
            kind: Kind::Module,
            locals: module_scope.bound,
            globals: BTreeSet::new(),
            nonlocals: BTreeSet::new(),
        }]),
        errors: RefCell::new(Vec::new()),
    };
    rewriter.visit_body(&mut module.body);
    let mut errors = rewriter.errors.into_inner();
    errors.sort_by_key(|error| error.range.start);
    errors
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Module,
    Function,
    Class,
    Comprehension,
}

struct Scope {
    kind: Kind,
    locals: BTreeSet<String>,
    globals: BTreeSet<String>,
    nonlocals: BTreeSet<String>,
}

impl Scope {
    fn new(kind: Kind, collected: Collected) -> Self {
        Self {
            kind,
            locals: collected.bound,
            globals: collected.globals,
            nonlocals: collected.nonlocals,
        }
    }
}

/// The names one scope body binds, without descending into nested scopes.
#[derive(Default)]
struct Collected {
    bound: BTreeSet<String>,
    imported: BTreeSet<String>,
    globals: BTreeSet<String>,
    nonlocals: BTreeSet<String>,
}

fn collect_scope(body: &[Stmt]) -> Collected {
    let mut collect = Collect::default();
    collect.visit_body(body);
    collect.0
}

fn collect_function(parameters: Option<&Parameters>, body: &[Stmt]) -> Collected {
    let mut collected = collect_scope(body);
    if let Some(parameters) = parameters {
        add_parameters(parameters, &mut collected.bound);
    }
    collected
}

fn add_parameters(parameters: &Parameters, bound: &mut BTreeSet<String>) {
    for parameter in parameters.iter() {
        bound.insert(parameter.name().to_string());
    }
}

fn type_param_names(type_params: Option<&TypeParams>) -> BTreeSet<String> {
    type_params
        .map(|params| {
            params
                .type_params
                .iter()
                .map(|param| match param {
                    TypeParam::TypeVar(var) => var.name.to_string(),
                    TypeParam::ParamSpec(spec) => spec.name.to_string(),
                    TypeParam::TypeVarTuple(tuple) => tuple.name.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Default)]
struct Collect(Collected);

impl<'a> Visitor<'a> for Collect {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                self.0.bound.insert(def.name.to_string());
            }
            Stmt::ClassDef(class) => {
                self.0.bound.insert(class.name.to_string());
            }
            Stmt::Global(global) => {
                self.0
                    .globals
                    .extend(global.names.iter().map(ToString::to_string));
            }
            Stmt::Nonlocal(nonlocal) => {
                self.0
                    .nonlocals
                    .extend(nonlocal.names.iter().map(ToString::to_string));
            }
            Stmt::Import(import) => {
                for alias in &import.names {
                    let name = match &alias.asname {
                        Some(asname) => asname.to_string(),
                        None => alias.name.split('.').next().unwrap_or_default().to_string(),
                    };
                    self.0.bound.insert(name.clone());
                    self.0.imported.insert(name);
                }
            }
            Stmt::ImportFrom(import) => {
                for alias in &import.names {
                    if alias.name.as_str() == "*" {
                        continue;
                    }
                    let name = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    self.0.bound.insert(name.clone());
                    self.0.imported.insert(name);
                }
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_except_handler(&mut self, handler: &'a ExceptHandler) {
        let ExceptHandler::ExceptHandler(handler_node) = handler;
        if let Some(name) = &handler_node.name {
            self.0.bound.insert(name.to_string());
        }
        visitor::walk_except_handler(self, handler);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) => {
                if matches!(name.ctx, ExprContext::Store | ExprContext::Del) {
                    self.0.bound.insert(name.id.to_string());
                }
            }
            Expr::Lambda(_) => {}
            Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {
                let mut walrus = Walrus(&mut self.0.bound);
                visitor::walk_expr(&mut walrus, expr);
            }
            _ => visitor::walk_expr(self, expr),
        }
    }

    fn visit_pattern(&mut self, pattern: &'a Pattern) {
        match pattern {
            Pattern::MatchAs(node) => {
                if let Some(name) = &node.name {
                    self.0.bound.insert(name.to_string());
                }
            }
            Pattern::MatchStar(node) => {
                if let Some(name) = &node.name {
                    self.0.bound.insert(name.to_string());
                }
            }
            Pattern::MatchMapping(node) => {
                if let Some(rest) = &node.rest {
                    self.0.bound.insert(rest.to_string());
                }
            }
            _ => {}
        }
        visitor::walk_pattern(self, pattern);
    }
}

/// The assignment-expression targets inside a comprehension: they bind in
/// the scope enclosing it.
struct Walrus<'b>(&'b mut BTreeSet<String>);

impl<'a> Visitor<'a> for Walrus<'_> {
    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Named(named) => {
                if let Expr::Name(target) = named.target.as_ref() {
                    self.0.insert(target.id.to_string());
                }
                self.visit_expr(&named.value);
            }
            Expr::Lambda(_) => {}
            _ => visitor::walk_expr(self, expr),
        }
    }
}

/// Every name any function in the module declares `global`, and whether the
/// module mentions `__name__`.
#[derive(Default)]
struct GlobalDecls {
    names: BTreeSet<String>,
    dunder_name: bool,
}

impl<'a> Visitor<'a> for GlobalDecls {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if let Stmt::Global(global) = stmt {
            self.names
                .extend(global.names.iter().map(ToString::to_string));
        }
        visitor::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Expr::Name(name) = expr
            && name.id.as_str() == "__name__"
        {
            self.dunder_name = true;
        }
        visitor::walk_expr(self, expr);
    }
}

struct Rewriter<'r> {
    rewrite: &'r dyn GlobalRewrite,
    scopes: RefCell<Vec<Scope>>,
    errors: RefCell<Vec<RewriteError>>,
}

impl Rewriter<'_> {
    /// Whether `name`, used in the innermost scope, resolves to the module
    /// scope.
    fn resolves_to_module(&self, name: &str) -> bool {
        let scopes = self.scopes.borrow();
        let innermost = scopes.len() - 1;
        for (index, scope) in scopes.iter().enumerate().rev() {
            match scope.kind {
                Kind::Module => return true,
                Kind::Class => {
                    // A class body's names are visible in the body itself
                    // only, never in a function or comprehension inside it.
                    if index == innermost && scope.locals.contains(name) {
                        return false;
                    }
                }
                Kind::Function => {
                    if scope.globals.contains(name) {
                        return true;
                    }
                    if scope.nonlocals.contains(name) || scope.locals.contains(name) {
                        return false;
                    }
                }
                Kind::Comprehension => {
                    if scope.locals.contains(name) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn refuse(&self, code: &'static str, message: String, range: TextRange) {
        self.errors.borrow_mut().push(RewriteError {
            code,
            message,
            range: range.start().into()..range.end().into(),
        });
    }

    /// Renames a binding identifier (a `def`/`class` name, an `except ...
    /// as` name, a pattern capture) when it binds a module-scope name.
    fn binding(&self, identifier: &mut Identifier) {
        if !self.resolves_to_module(identifier.as_str()) {
            return;
        }
        match self.rewrite.name(identifier.as_str(), ExprContext::Store) {
            NameRewrite::Keep => {}
            NameRewrite::Rename(name) => identifier.id = Name::new(name),
            NameRewrite::Refuse { code, message } => self.refuse(code, message, identifier.range),
        }
    }

    fn push(&self, scope: Scope) {
        self.scopes.borrow_mut().push(scope);
    }

    fn pop(&self) {
        self.scopes.borrow_mut().pop();
    }

    /// A type-parameter scope (PEP 695), when `names` is non-empty.
    fn push_type_params(&self, names: BTreeSet<String>) -> bool {
        if names.is_empty() {
            return false;
        }
        self.push(Scope {
            kind: Kind::Function,
            locals: names,
            globals: BTreeSet::new(),
            nonlocals: BTreeSet::new(),
        });
        true
    }

    /// Rewrites a module-scope attribute chain rooted at `expr`; `false`
    /// when the chain is not one the rewrite handles.
    fn chain(&self, expr: &mut Expr) -> bool {
        let mut attrs: Vec<String> = Vec::new();
        let mut current: &Expr = expr;
        while let Expr::Attribute(attribute) = current {
            attrs.push(attribute.attr.to_string());
            current = &attribute.value;
        }
        let Expr::Name(root) = current else {
            return false;
        };
        let root = root.id.to_string();
        if !self.resolves_to_module(&root) {
            return false;
        }
        attrs.reverse();
        let Expr::Attribute(outer) = &*expr else {
            return false;
        };
        let ctx = outer.ctx;
        let borrowed: Vec<&str> = attrs.iter().map(String::as_str).collect();
        match self.rewrite.chain(&root, &borrowed, ctx) {
            None => false,
            Some(ChainRewrite::Refuse { code, message }) => {
                self.refuse(code, message, expr.range());
                true
            }
            Some(ChainRewrite::Name {
                consumed,
                name,
                cell,
            }) => {
                let keep = attrs.len() - consumed;
                let mut target: &mut Expr = expr;
                for _ in 0..keep {
                    let Expr::Attribute(attribute) = target else {
                        unreachable!("the chain was measured above");
                    };
                    target = attribute.value.as_mut();
                }
                let range = target.range();
                let ctx = if keep == 0 { ctx } else { ExprContext::Load };
                *target = if cell {
                    super::cell_element(&name, ctx, range)
                } else {
                    Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::default(),
                        range,
                        id: Name::new(name),
                        ctx,
                    })
                };
                true
            }
        }
    }

    fn comprehension(&self, generators: &mut [Comprehension], elements: &mut [&mut Expr]) {
        let Some((first, _)) = generators.split_first_mut() else {
            return;
        };
        self.visit_expr(&mut first.iter);
        let mut locals = BTreeSet::new();
        for generator in generators.iter() {
            let mut targets = Collect::default();
            targets.visit_expr(&generator.target);
            locals.extend(targets.0.bound);
        }
        self.push(Scope {
            kind: Kind::Comprehension,
            locals,
            globals: BTreeSet::new(),
            nonlocals: BTreeSet::new(),
        });
        for (index, generator) in generators.iter_mut().enumerate() {
            self.visit_expr(&mut generator.target);
            if index > 0 {
                self.visit_expr(&mut generator.iter);
            }
            for condition in &mut generator.ifs {
                self.visit_expr(condition);
            }
        }
        for element in elements {
            self.visit_expr(element);
        }
        self.pop();
    }

    /// Rewrites module-scope names inside the nested string annotations of
    /// `annotation` (`list["Point"]`): a string naming a renamed global is
    /// replaced by the expression it contains. A `Literal[...]` subscript
    /// keeps its strings.
    fn string_annotations(&self, annotation: &mut Expr) {
        struct Strings<'s, 'r>(&'s Rewriter<'r>);
        impl Transformer for Strings<'_, '_> {
            fn visit_expr(&self, expr: &mut Expr) {
                match expr {
                    Expr::Subscript(subscript) if is_literal(&subscript.value) => {}
                    Expr::StringLiteral(literal) => {
                        let Ok(mut parsed) = crate::parse_string_annotation(literal) else {
                            return;
                        };
                        if self.0.renames_any(&parsed) {
                            self.visit_expr(&mut parsed);
                            *expr = parsed;
                        }
                    }
                    _ => tr::walk_expr(self, expr),
                }
            }
        }
        Strings(self).visit_expr(annotation);
    }

    fn renames_any(&self, expr: &Expr) -> bool {
        struct Names<'n>(&'n mut Vec<String>);
        impl<'a> Visitor<'a> for Names<'_> {
            fn visit_expr(&mut self, expr: &'a Expr) {
                if let Expr::Name(name) = expr {
                    self.0.push(name.id.to_string());
                }
                visitor::walk_expr(self, expr);
            }
        }
        let mut names = Vec::new();
        Names(&mut names).visit_expr(expr);
        names.iter().any(|name| {
            self.resolves_to_module(name)
                && matches!(
                    self.rewrite.name(name, ExprContext::Load),
                    NameRewrite::Rename(_)
                )
        })
    }
}

fn parameter_defaults(parameters: &mut Parameters) -> impl Iterator<Item = &mut Expr> {
    parameters
        .posonlyargs
        .iter_mut()
        .chain(parameters.args.iter_mut())
        .chain(parameters.kwonlyargs.iter_mut())
        .filter_map(|parameter| parameter.default.as_deref_mut())
}

fn parameter_annotations(parameters: &mut Parameters) -> impl Iterator<Item = &mut Expr> {
    let Parameters {
        posonlyargs,
        args,
        vararg,
        kwonlyargs,
        kwarg,
        ..
    } = parameters;
    posonlyargs
        .iter_mut()
        .chain(args.iter_mut())
        .chain(kwonlyargs.iter_mut())
        .map(|parameter| &mut parameter.parameter)
        .chain(vararg.as_deref_mut())
        .chain(kwarg.as_deref_mut())
        .filter_map(|parameter| parameter.annotation.as_deref_mut())
}

fn is_literal(expr: &Expr) -> bool {
    match expr {
        Expr::Name(name) => name.id.as_str() == "Literal",
        Expr::Attribute(attribute) => attribute.attr.as_str() == "Literal",
        _ => false,
    }
}

impl Transformer for Rewriter<'_> {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                for decorator in &mut def.decorator_list {
                    self.visit_decorator(decorator);
                }
                for default in parameter_defaults(&mut def.parameters) {
                    self.visit_expr(default);
                }
                let type_params = type_param_names(def.type_params.as_deref());
                let pushed = self.push_type_params(type_params.clone());
                for annotation in parameter_annotations(&mut def.parameters) {
                    self.visit_annotation(annotation);
                }
                if let Some(returns) = &mut def.returns {
                    self.visit_annotation(returns);
                }
                if pushed {
                    self.pop();
                }
                self.binding(&mut def.name);
                let mut collected = collect_function(Some(&def.parameters), &def.body);
                collected.bound.extend(type_params);
                self.push(Scope::new(Kind::Function, collected));
                self.visit_body(&mut def.body);
                self.pop();
            }
            Stmt::ClassDef(class) => {
                for decorator in &mut class.decorator_list {
                    self.visit_decorator(decorator);
                }
                let pushed = self.push_type_params(type_param_names(class.type_params.as_deref()));
                if let Some(arguments) = &mut class.arguments {
                    self.visit_arguments(arguments);
                }
                self.binding(&mut class.name);
                self.push(Scope::new(Kind::Class, collect_scope(&class.body)));
                self.visit_body(&mut class.body);
                self.pop();
                if pushed {
                    self.pop();
                }
            }
            Stmt::Global(global) => {
                for name in &mut global.names {
                    if let NameRewrite::Rename(renamed) =
                        self.rewrite.name(name.as_str(), ExprContext::Invalid)
                    {
                        name.id = Name::new(renamed);
                    }
                }
            }
            Stmt::Nonlocal(_) | Stmt::Import(_) | Stmt::ImportFrom(_) => {}
            Stmt::Assign(assign) => {
                // `T = TypeVar("T")`, `Color = Enum("Color", ...)`: the
                // string names the variable, so it follows the rename.
                let before = match assign.targets.as_slice() {
                    [Expr::Name(target)] => Some(target.id.to_string()),
                    _ => None,
                };
                tr::walk_stmt(self, stmt);
                let Stmt::Assign(assign) = stmt else {
                    unreachable!("still the assignment");
                };
                if let (Some(before), [Expr::Name(target)]) = (before, assign.targets.as_slice())
                    && target.id.as_str() != before
                    && let Expr::Call(call) = assign.value.as_mut()
                    && let Some(Expr::StringLiteral(literal)) = call.arguments.args.first_mut()
                    && literal.value.to_str() == before
                {
                    let renamed = target.id.to_string();
                    let range = literal.range;
                    *literal = ruff_python_ast::ExprStringLiteral {
                        node_index: AtomicNodeIndex::default(),
                        range,
                        value: ruff_python_ast::StringLiteralValue::single(
                            ruff_python_ast::StringLiteral {
                                range,
                                node_index: AtomicNodeIndex::default(),
                                value: renamed.into(),
                                flags: ruff_python_ast::StringLiteralFlags::empty(),
                            },
                        ),
                    };
                }
            }
            _ => tr::walk_stmt(self, stmt),
        }
    }

    fn visit_annotation(&self, expr: &mut Expr) {
        self.string_annotations(expr);
        self.visit_expr(expr);
    }

    fn visit_except_handler(&self, handler: &mut ExceptHandler) {
        let ExceptHandler::ExceptHandler(node) = handler;
        if let Some(name) = &mut node.name {
            self.binding(name);
        }
        tr::walk_except_handler(self, handler);
    }

    fn visit_pattern(&self, pattern: &mut Pattern) {
        match pattern {
            Pattern::MatchAs(node) => {
                if let Some(name) = &mut node.name {
                    self.binding(name);
                }
            }
            Pattern::MatchStar(node) => {
                if let Some(name) = &mut node.name {
                    self.binding(name);
                }
            }
            Pattern::MatchMapping(node) => {
                if let Some(rest) = &mut node.rest {
                    self.binding(rest);
                }
            }
            _ => {}
        }
        tr::walk_pattern(self, pattern);
    }

    fn visit_type_params(&self, _type_params: &mut TypeParams) {
        // Handled by the `def`/`class` arms, which scope them.
    }

    fn visit_expr(&self, expr: &mut Expr) {
        match expr {
            Expr::Name(name) => {
                if !self.resolves_to_module(name.id.as_str()) {
                    return;
                }
                match self.rewrite.name(name.id.as_str(), name.ctx) {
                    NameRewrite::Keep => {}
                    NameRewrite::Rename(renamed) => name.id = Name::new(renamed),
                    NameRewrite::Refuse { code, message } => self.refuse(code, message, name.range),
                }
            }
            Expr::Attribute(_) => {
                if !self.chain(expr) {
                    tr::walk_expr(self, expr);
                }
            }
            Expr::Lambda(lambda) => {
                if let Some(parameters) = &mut lambda.parameters {
                    for default in parameter_defaults(parameters) {
                        self.visit_expr(default);
                    }
                }
                let mut collected = Collected::default();
                if let Some(parameters) = &lambda.parameters {
                    add_parameters(parameters, &mut collected.bound);
                }
                Walrus(&mut collected.bound).visit_expr(&lambda.body);
                self.push(Scope::new(Kind::Function, collected));
                self.visit_expr(&mut lambda.body);
                self.pop();
            }
            Expr::ListComp(comp) => {
                self.comprehension(&mut comp.generators, &mut [comp.elt.as_mut()]);
            }
            Expr::SetComp(comp) => {
                self.comprehension(&mut comp.generators, &mut [comp.elt.as_mut()]);
            }
            Expr::Generator(comp) => {
                self.comprehension(&mut comp.generators, &mut [comp.elt.as_mut()]);
            }
            Expr::DictComp(comp) => {
                let mut elements: Vec<&mut Expr> = comp
                    .key
                    .as_deref_mut()
                    .into_iter()
                    .chain([comp.value.as_mut()])
                    .collect();
                self.comprehension(&mut comp.generators, &mut elements);
            }
            _ => tr::walk_expr(self, expr),
        }
    }
}
