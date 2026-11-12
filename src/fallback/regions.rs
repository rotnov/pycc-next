//! Regions of a selected unit (docs/design/selected-regions.md, section
//! 4.1).
//!
//! A call in a selected function's body is a *region edge* when it names
//! another selected function (or the function itself) statically:
//!
//! - `f(...)`, `f` a module global no other code can rebind: the module binds
//!   it once, with its own undecorated module-level `def` (no other binding
//!   at module scope, no `del`, no wildcard import), and no scope declares it
//!   `global`;
//! - `self.m(...)`, `self` the first parameter of an undecorated method that
//!   does not rebind it (not `__new__`, `__init_subclass__` or
//!   `__class_getitem__`, implicitly static or class methods whose first
//!   parameter is no instance), and `m` a method of the same class
//!   statement that its body binds once, with its own undecorated `def`
//!   (the method the direct-call cell of
//!   `crates/pycc_codegen/src/direct_call.rs` reaches, unless a subclass or
//!   an instance overrides it). A nested class is that class only when its
//!   enclosing class's body binds its name once, with its `class`.
//!
//! A callee with `*args`, `**kwargs` or a `__class__` cell is no edge: its
//! direct-call cell stays zero, so no compiled call reaches its body
//! directly. A call in the body of a lambda or a nested `def` or `class` is
//! not the compiled body's: CPython runs those. The decorators and
//! parameter defaults of a nested `def` or lambda, and the decorators,
//! bases and keywords of a nested `class`, are the compiled body's: their
//! calls count.
//!
//! A *region* is a connected component of region edges among the selected
//! functions that has at least one edge: a function no edge touches is in
//! no region, and a function that only calls itself is a region of one.

use super::*;
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::Expr;

impl Plan {
    /// The selected units `units` grouped into regions: each region's
    /// members in source order, the regions in the order of their first
    /// member.
    pub(super) fn regions(&self, units: &[usize]) -> Vec<Vec<usize>> {
        let edges = self.region_edges(units);
        let mut parent: BTreeMap<usize, usize> = units.iter().map(|u| (*u, *u)).collect();
        fn find(parent: &mut BTreeMap<usize, usize>, mut u: usize) -> usize {
            while parent[&u] != u {
                let up = parent[&parent[&u]];
                parent.insert(u, up);
                u = up;
            }
            u
        }
        for (a, b) in &edges {
            let (ra, rb) = (find(&mut parent, *a), find(&mut parent, *b));
            parent.insert(ra.max(rb), ra.min(rb));
        }
        let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for unit in units {
            if edges.iter().any(|(a, b)| a == unit || b == unit) {
                let root = find(&mut parent, *unit);
                by_root.entry(root).or_default().push(*unit);
            }
        }
        let start = |unit: &usize| self.lift(*unit).map_or(0, |lift| lift.range.start);
        let mut out: Vec<Vec<usize>> = by_root
            .into_values()
            .map(|mut members| {
                members.sort_by_key(start);
                members
            })
            .collect();
        out.sort_by_key(|members| start(&members[0]));
        out
    }

    /// The region edges among the selected units `units`: (caller, callee).
    pub(super) fn region_edges(&self, units: &[usize]) -> BTreeSet<(usize, usize)> {
        // The selected functions a call can name statically.
        let mut globals: BTreeMap<&str, usize> = BTreeMap::new();
        // A method by its class's top-level statement, dotted path and name:
        // two classes of one path (two `class C:` statements) are two.
        let mut methods: BTreeMap<(usize, &str, &str), usize> = BTreeMap::new();
        for unit in units {
            let Some(lift) = self.lift(*unit) else {
                continue;
            };
            if !self.directly_callable(*unit) {
                continue;
            }
            match lift.qualname.rsplit_once('.') {
                None if matches!(&self.ast[lift.owner], Stmt::FunctionDef(def) if def.name.as_str() == lift.qualname)
                    && self.module_binds_once(&lift.qualname) =>
                {
                    globals.insert(&lift.qualname, *unit);
                }
                Some((owner, member)) if self.class_binds_once(lift.owner, owner, member) => {
                    methods.insert((lift.owner, owner, member), *unit);
                }
                _ => {}
            }
        }
        let mut edges = BTreeSet::new();
        for unit in units {
            let (Some(lift), Stmt::FunctionDef(def)) = (self.lift(*unit), &self.ast[*unit]) else {
                continue;
            };
            let mut locals = names::bound_names(&def.body);
            locals.extend(lift.params.iter().cloned());
            // A method's receiver, when it is one: an undecorated method's
            // first parameter, never rebound, unless the method is
            // implicitly static or a class method.
            let (class, member) = match lift.qualname.rsplit_once('.') {
                Some((owner, member)) => (Some(owner), member),
                None => (None, lift.qualname.as_str()),
            };
            let receiver = class
                .filter(|_| !self.decorated(*unit))
                .filter(|_| !IMPLICIT_NON_INSTANCE.contains(&member))
                .and_then(|_| lift.params.first())
                .filter(|first| !names::bound_names(&def.body).contains(*first))
                .map(String::as_str);
            let mut calls = Calls {
                locals: &locals,
                shadowed: Vec::new(),
                receiver,
                globals: Vec::new(),
                methods: Vec::new(),
            };
            for stmt in &def.body {
                calls.visit_stmt(stmt);
            }
            for name in &calls.globals {
                if let Some(callee) = globals.get(name.as_str()) {
                    edges.insert((*unit, *callee));
                }
            }
            if let Some(class) = class {
                for member in &calls.methods {
                    if let Some(callee) = methods.get(&(lift.owner, class, member.as_str())) {
                        edges.insert((*unit, *callee));
                    }
                }
            }
        }
        edges
    }

    /// Whether the lift `unit` has decorators.
    fn decorated(&self, unit: usize) -> bool {
        self.lift(unit)
            .is_some_and(|lift| self.source[lift.range.start as usize..].starts_with('@'))
    }

    /// Whether a compiled call can reach the lift `unit`'s body directly:
    /// undecorated, with no `*args`, `**kwargs` or `__class__` cell.
    fn directly_callable(&self, unit: usize) -> bool {
        let (Some(lift), Stmt::FunctionDef(def)) = (self.lift(unit), &self.ast[unit]) else {
            return false;
        };
        !self.decorated(unit)
            && !lift.uses_class
            && def.parameters.vararg.is_none()
            && def.parameters.kwarg.is_none()
    }

    /// Whether the module binds `name` in exactly one top-level statement,
    /// no scope declares it `global` and no wildcard import may bind it.
    fn module_binds_once(&self, name: &str) -> bool {
        let top = &self.stmts[..self.top];
        top.iter().filter(|stmt| stmt.bound.contains(name)).count() == 1
            && !top.iter().any(|stmt| stmt.bound.contains("*"))
            && !self.stmts.iter().any(|stmt| stmt.declares.contains(name))
    }

    /// Whether the class `path` (dotted, from the top-level statement
    /// `owner`) binds `member` in exactly one statement of its body, its
    /// `def`, and each class of `path` below the first is bound once in its
    /// enclosing class's body, by its `class`.
    fn class_binds_once(&self, owner: usize, path: &str, member: &str) -> bool {
        let mut parts = path.split('.');
        let Stmt::ClassDef(class) = &self.ast[owner] else {
            return false;
        };
        let mut class: &pycc_ast::StmtClassDef = class;
        if parts.next() != Some(class.name.as_str()) {
            return false;
        }
        for part in parts {
            let Some(Stmt::ClassDef(inner)) = sole_binder(&class.body, part) else {
                return false;
            };
            class = inner;
        }
        matches!(sole_binder(&class.body, member), Some(Stmt::FunctionDef(def)) if def.name.as_str() == member)
    }
}

/// Methods whose first parameter is no instance: `__new__` is implicitly a
/// static method, `__init_subclass__` and `__class_getitem__` implicitly
/// class methods.
const IMPLICIT_NON_INSTANCE: [&str; 3] = ["__new__", "__init_subclass__", "__class_getitem__"];

/// The one statement of `body` that binds `name`, if exactly one does and
/// it is a `def` or `class` of that name.
fn sole_binder<'b>(body: &'b [Stmt], name: &str) -> Option<&'b Stmt> {
    let mut binders = body
        .iter()
        .filter(|stmt| names::bound_names(std::slice::from_ref(*stmt)).contains(name));
    match (binders.next(), binders.next()) {
        (Some(stmt @ Stmt::FunctionDef(def)), None) if def.name.as_str() == name => Some(stmt),
        (Some(stmt @ Stmt::ClassDef(class)), None) if class.name.as_str() == name => Some(stmt),
        _ => None,
    }
}

/// The calls a compiled body makes by a module global's name, and by a
/// method of its receiver.
struct Calls<'l> {
    /// The body's own names: a call of one is not a module global's.
    locals: &'l BTreeSet<String>,
    /// The targets of the comprehensions being visited.
    shadowed: Vec<BTreeSet<String>>,
    receiver: Option<&'l str>,
    globals: Vec<String>,
    methods: Vec<String>,
}

impl Calls<'_> {
    fn is_global(&self, name: &str) -> bool {
        !self.locals.contains(name) && !self.shadowed.iter().any(|scope| scope.contains(name))
    }
}

impl<'a> Visitor<'a> for Calls<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        // A nested scope's body is CPython's; what the statement evaluates
        // to create it is the compiled body's.
        match stmt {
            Stmt::FunctionDef(def) => {
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                self.visit_parameters(&def.parameters);
            }
            Stmt::ClassDef(class) => {
                for decorator in &class.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(arguments) = &class.arguments {
                    self.visit_arguments(arguments);
                }
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    // Annotations are evaluated lazily (PEP 649), not by the compiled body.
    fn visit_annotation(&mut self, _expr: &'a Expr) {}

    fn visit_expr(&mut self, expr: &'a Expr) {
        let generators = match expr {
            Expr::Lambda(lambda) => {
                // Its defaults are the compiled body's; its body is not.
                if let Some(parameters) = &lambda.parameters {
                    self.visit_parameters(parameters);
                }
                return;
            }
            Expr::Call(call) => {
                match call.func.as_ref() {
                    Expr::Name(name) if self.is_global(name.id.as_str()) => {
                        self.globals.push(name.id.to_string());
                    }
                    Expr::Attribute(attribute) => {
                        if let Expr::Name(value) = attribute.value.as_ref()
                            && Some(value.id.as_str()) == self.receiver
                            && !self.shadowed.iter().any(|scope| scope.contains(value.id.as_str()))
                        {
                            self.methods.push(attribute.attr.to_string());
                        }
                    }
                    _ => {}
                }
                None
            }
            Expr::ListComp(e) => Some(&e.generators),
            Expr::SetComp(e) => Some(&e.generators),
            Expr::DictComp(e) => Some(&e.generators),
            Expr::Generator(e) => Some(&e.generators),
            _ => None,
        };
        match generators {
            Some(generators) => {
                self.shadowed.push(
                    generators
                        .iter()
                        .flat_map(|generator| pycc_ast::namespace::target_names(&generator.target))
                        .collect(),
                );
                visitor::walk_expr(self, expr);
                self.shadowed.pop();
            }
            None => visitor::walk_expr(self, expr),
        }
    }
}

/// The regions [`Plan::regions`] forms from the functions `selected`
/// (qualified name, first line) of the module `source`, by the same keys,
/// or none when `source` does not parse.
pub(crate) fn regions_of(source: &str, selected: &BTreeSet<crate::select::Key>) -> Vec<Vec<crate::select::Key>> {
    let publishes = Publishes {
        functions: true,
        classes: true,
        infers_unannotated: false,
        variables: true,
        foreign_relative_imports: true,
    };
    let Some(plan) = Plan::new(source, "<regions>", publishes) else {
        return Vec::new();
    };
    let key = |lift: &lift::Lift| (lift.qualname.clone(), line_of(&plan.source, lift.range.start) as u32);
    let units: Vec<usize> = (0..plan.lifts.len())
        .map(|at| plan.top + at)
        .filter(|unit| plan.lift(*unit).is_some_and(|lift| selected.contains(&key(lift))))
        .collect();
    plan.regions(&units)
        .into_iter()
        .map(|members| members.iter().filter_map(|unit| plan.lift(*unit)).map(key).collect())
        .collect()
}

/// The entries of the region `members`: each member a profile counted
/// called from outside the region (`calls`: all its calls, less those its
/// region's members made), or that no profile counted. Static edges cannot
/// tell this, since a module's bytecode may call any member.
///
/// Approximate: a member's `total` and each caller's count are medians
/// taken separately across the profile's runs, so they need not add up as
/// in any one run. A guard (R5's entry guard)
/// must not rely on a member being no entry, nor on `entries: none`.
pub(super) fn entries<'k>(
    members: &'k [crate::select::Key],
    calls: &BTreeMap<crate::select::Key, crate::select::Calls>,
) -> Vec<&'k crate::select::Key> {
    members
        .iter()
        .filter(|member| {
            calls.get(*member).is_none_or(|c| {
                let inside: u64 = members.iter().filter_map(|caller| c.from.get(caller)).sum();
                c.total > inside
            })
        })
        .collect()
}

/// The `--report-fallbacks` line of the region `members` of `module`:
/// `region: MODULE: A, B (entries: A)`.
pub(super) fn report_line(
    module: &str,
    members: &[crate::select::Key],
    calls: &BTreeMap<crate::select::Key, crate::select::Calls>,
) -> String {
    let names: Vec<&str> = members.iter().map(|(q, _)| q.as_str()).collect();
    let entries: Vec<&str> = entries(members, calls).into_iter().map(|(q, _)| q.as_str()).collect();
    format!(
        "region: {module}: {} (entries: {})",
        names.join(", "),
        if entries.is_empty() { "none".to_string() } else { entries.join(", ") }
    )
}

#[cfg(test)]
#[path = "regions_tests.rs"]
mod tests;
