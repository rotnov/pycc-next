//! Item access on an instance of a class that defines the item dunders:
//! `x[k]` is `x.__getitem__(k)`, `x[k] = v` is `x.__setitem__(k, v)`,
//! `x[k] op= v` is `x.__setitem__(k, x.__getitem__(k) op v)` and
//! `del x[k]` is `x.__delitem__(k)` -- the methods CPython's item protocol
//! calls on `type(x)`.
//!
//! `x` is known to be such an instance only by name: `self` (the first
//! parameter of a method of a class whose module-level MRO defines the
//! dunder), or a name every binding of which in its scope is a call of
//! such a class (`dsu = DSU()`). A class qualifies only when every base is
//! another class of the module, so no builtin base supplies the protocol.
//! A slice index (`x[a:b]`) is left alone, and so is a store whose key and
//! value both have side effects: CPython evaluates the value first, the
//! method call the key first.

use super::aliases::rebound_names;
use super::build::{attribute, call};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{AtomicNodeIndex, Expr, ExprContext, Stmt, StmtExpr};
use std::collections::{HashMap, HashSet};

const GET: &str = "__getitem__";
const SET: &str = "__setitem__";
const DEL: &str = "__delitem__";

/// Rewrites every such item access (module doc).
pub(crate) fn instance_item_access(body: &mut [Stmt]) {
    let classes = item_classes(body);
    if classes.is_empty() {
        return;
    }
    let top_level: Vec<Stmt> = body
        .iter()
        .filter(|stmt| !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)))
        .cloned()
        .collect();
    // A module-level name counts every binding in the module, functions
    // included: a `global` statement can rebind it there.
    let module_names = instance_names(&top_level, &rebound_names(body), &classes, &HashSet::new());
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::FunctionDef(def) => {
                let names = function_names(def, &classes, &module_names, None);
                Rewrite { names }.block(&mut def.body);
            }
            Stmt::ClassDef(class) => {
                let dunders = classes.get(class.name.as_str()).cloned();
                for item in class.body.iter_mut() {
                    if let Stmt::FunctionDef(def) = item {
                        let names = function_names(def, &classes, &module_names, dunders.as_ref());
                        Rewrite { names }.block(&mut def.body);
                    }
                }
            }
            _ => Rewrite {
                names: module_names.clone(),
            }
            .block(std::slice::from_mut(stmt)),
        }
    }
}

/// Each module-level class whose module-level MRO defines at least one
/// item dunder, with the dunders it has.
fn item_classes(body: &[Stmt]) -> HashMap<String, HashSet<&'static str>> {
    let defs: HashMap<&str, &ruff_python_ast::StmtClassDef> = body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::ClassDef(class) => Some((class.name.as_str(), class)),
            _ => None,
        })
        .collect();
    let rebound = rebound_names(body);
    let mut found = HashMap::new();
    for name in defs.keys() {
        if rebound.get(*name).copied().unwrap_or(0) != 1 {
            continue;
        }
        if let Some(dunders) = mro_dunders(name, &defs, &mut Vec::new())
            && !dunders.is_empty()
        {
            found.insert((*name).to_string(), dunders);
        }
    }
    found
}

/// The item dunders `name`'s class and its bases define, or `None` when a
/// base is not a plain class of the module (or the hierarchy is cyclic).
fn mro_dunders(
    name: &str,
    defs: &HashMap<&str, &ruff_python_ast::StmtClassDef>,
    seen: &mut Vec<String>,
) -> Option<HashSet<&'static str>> {
    if seen.iter().any(|s| s == name) {
        return None;
    }
    seen.push(name.to_string());
    let class = defs.get(name)?;
    if class
        .arguments
        .as_ref()
        .is_some_and(|args| !args.keywords.is_empty())
        || !class.decorator_list.is_empty()
    {
        return None;
    }
    let mut dunders = HashSet::new();
    for item in &class.body {
        if let Stmt::FunctionDef(def) = item {
            for dunder in [GET, SET, DEL] {
                if def.name.as_str() == dunder {
                    dunders.insert(dunder);
                }
            }
        }
    }
    for base in class.bases() {
        match base {
            Expr::Name(base) if base.id.as_str() == "object" => {}
            Expr::Name(base) => dunders.extend(mro_dunders(base.id.as_str(), defs, seen)?),
            _ => return None,
        }
    }
    seen.pop();
    Some(dunders)
}

/// The names of `body`'s scope bound only to instances of an item class:
/// every binding `rebound` counts is a `name = C(...)` of `body`.
fn instance_names(
    body: &[Stmt],
    rebound: &HashMap<String, usize>,
    classes: &HashMap<String, HashSet<&'static str>>,
    excluded: &HashSet<String>,
) -> HashMap<String, HashSet<&'static str>> {
    let mut bound: HashMap<String, (usize, Option<String>)> = HashMap::new();
    collect_constructions(body, classes, &mut bound);
    bound
        .into_iter()
        .filter(|(name, (count, class))| {
            class.is_some()
                && rebound.get(name).copied().unwrap_or(0) == *count
                && !excluded.contains(name)
        })
        .filter_map(|(name, (_, class))| {
            let dunders = classes.get(class.as_ref()?)?.clone();
            Some((name, dunders))
        })
        .collect()
}

/// Counts each `name = C(...)` assignment of an item class `C` in `body`
/// (nested blocks included, nested scopes not); a name assigned calls of
/// two classes records none.
fn collect_constructions(
    body: &[Stmt],
    classes: &HashMap<String, HashSet<&'static str>>,
    bound: &mut HashMap<String, (usize, Option<String>)>,
) {
    for stmt in body {
        match stmt {
            Stmt::Assign(assign) => {
                if let [Expr::Name(target)] = assign.targets.as_slice()
                    && let Expr::Call(value) = assign.value.as_ref()
                    && let Expr::Name(callee) = value.func.as_ref()
                    && classes.contains_key(callee.id.as_str())
                {
                    let entry = bound
                        .entry(target.id.to_string())
                        .or_insert((0, Some(callee.id.to_string())));
                    entry.0 += 1;
                    if entry.1.as_deref() != Some(callee.id.as_str()) {
                        entry.1 = None;
                    }
                }
            }
            Stmt::If(s) => {
                collect_constructions(&s.body, classes, bound);
                for clause in &s.elif_else_clauses {
                    collect_constructions(&clause.body, classes, bound);
                }
            }
            Stmt::For(s) => {
                collect_constructions(&s.body, classes, bound);
                collect_constructions(&s.orelse, classes, bound);
            }
            Stmt::While(s) => {
                collect_constructions(&s.body, classes, bound);
                collect_constructions(&s.orelse, classes, bound);
            }
            Stmt::With(s) => collect_constructions(&s.body, classes, bound),
            Stmt::Try(s) => {
                collect_constructions(&s.body, classes, bound);
                for handler in &s.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    collect_constructions(&h.body, classes, bound);
                }
                collect_constructions(&s.orelse, classes, bound);
                collect_constructions(&s.finalbody, classes, bound);
            }
            _ => {}
        }
    }
}

/// The instance names a function body reads: its own, the module's it does
/// not bind, and, for a method of an item class, its `self` parameter.
fn function_names(
    def: &ruff_python_ast::StmtFunctionDef,
    classes: &HashMap<String, HashSet<&'static str>>,
    module_names: &HashMap<String, HashSet<&'static str>>,
    method_of: Option<&HashSet<&'static str>>,
) -> HashMap<String, HashSet<&'static str>> {
    let params: HashSet<String> = def
        .parameters
        .iter()
        .map(|p| p.name().to_string())
        .collect();
    let local = rebound_names(&def.body);
    let mut names: HashMap<String, HashSet<&'static str>> = module_names
        .iter()
        .filter(|(name, _)| !params.contains(*name) && !local.contains_key(*name))
        .map(|(name, dunders)| (name.clone(), dunders.clone()))
        .collect();
    names.extend(instance_names(&def.body, &local, classes, &params));
    if let Some(dunders) = method_of
        && def.decorator_list.is_empty()
        && let Some(first) = def.parameters.args.first()
        && !local.contains_key(first.parameter.name.as_str())
    {
        names.insert(first.parameter.name.to_string(), dunders.clone());
    }
    names
}

struct Rewrite {
    /// Instance names, with the item dunders their class defines.
    names: HashMap<String, HashSet<&'static str>>,
}

impl Rewrite {
    /// `base` when it is an instance name whose class defines `dunder`.
    fn receiver(&self, base: &Expr, dunder: &str) -> bool {
        matches!(base, Expr::Name(name) if self.names.get(name.id.as_str()).is_some_and(|d| d.contains(dunder)))
    }

    fn method_call(
        base: &Expr,
        dunder: &str,
        args: Vec<Expr>,
        range: ruff_text_size::TextRange,
    ) -> Expr {
        let mut base = base.clone();
        if let Expr::Name(name) = &mut base {
            name.ctx = ExprContext::Load;
        }
        call(attribute(base, dunder, range), args, range)
    }

    fn expr_stmt(value: Expr) -> Stmt {
        Stmt::Expr(StmtExpr {
            node_index: AtomicNodeIndex::default(),
            range: ruff_text_size::Ranged::range(&value),
            value: Box::new(value),
        })
    }

    fn block(&self, body: &mut [Stmt]) {
        for stmt in body.iter_mut() {
            self.visit_stmt(stmt);
        }
    }

    /// The statement `stmt` becomes, when it stores into or deletes an item.
    fn item_statement(&self, stmt: &Stmt) -> Option<Stmt> {
        match stmt {
            Stmt::Assign(assign) => {
                let [Expr::Subscript(target)] = assign.targets.as_slice() else {
                    return None;
                };
                if !self.receiver(&target.value, SET)
                    || matches!(target.slice.as_ref(), Expr::Slice(_))
                    || !(super::is_simple(&target.slice) || super::is_simple(&assign.value))
                {
                    return None;
                }
                Some(Self::expr_stmt(Self::method_call(
                    &target.value,
                    SET,
                    vec![(*target.slice).clone(), (*assign.value).clone()],
                    assign.range,
                )))
            }
            Stmt::AugAssign(aug) => {
                let Expr::Subscript(target) = aug.target.as_ref() else {
                    return None;
                };
                if !self.receiver(&target.value, SET)
                    || !self.receiver(&target.value, GET)
                    || !super::is_simple(&target.slice)
                {
                    return None;
                }
                let current =
                    Self::method_call(&target.value, GET, vec![(*target.slice).clone()], aug.range);
                let updated = Expr::BinOp(ruff_python_ast::ExprBinOp {
                    node_index: AtomicNodeIndex::default(),
                    range: aug.range,
                    left: Box::new(current),
                    op: aug.op,
                    right: aug.value.clone(),
                });
                Some(Self::expr_stmt(Self::method_call(
                    &target.value,
                    SET,
                    vec![(*target.slice).clone(), updated],
                    aug.range,
                )))
            }
            Stmt::Delete(delete) => {
                let [Expr::Subscript(target)] = delete.targets.as_slice() else {
                    return None;
                };
                if !self.receiver(&target.value, DEL)
                    || matches!(target.slice.as_ref(), Expr::Slice(_))
                {
                    return None;
                }
                Some(Self::expr_stmt(Self::method_call(
                    &target.value,
                    DEL,
                    vec![(*target.slice).clone()],
                    delete.range,
                )))
            }
            _ => None,
        }
    }
}

impl Transformer for Rewrite {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        if let Some(rewritten) = self.item_statement(stmt) {
            *stmt = rewritten;
        }
        match stmt {
            // A nested scope reads its own names.
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
            _ => transformer::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Lambda(_) = expr {
            return;
        }
        transformer::walk_expr(self, expr);
        if let Expr::Subscript(subscript) = expr
            && subscript.ctx == ExprContext::Load
            && !matches!(subscript.slice.as_ref(), Expr::Slice(_))
            && self.receiver(&subscript.value, GET)
        {
            *expr = Self::method_call(
                &subscript.value,
                GET,
                vec![(*subscript.slice).clone()],
                subscript.range,
            );
        }
    }
}
