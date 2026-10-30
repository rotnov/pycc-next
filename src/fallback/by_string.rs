//! Fallback code reaching module globals by a string (issue #20).
//!
//! `eval("c")`, `exec(src)` and `globals()["c"]` read (and bind) module
//! globals no static reference names, so the name analysis (`names`) would
//! leave `c` a native global CPython cannot see. A unit whose code can do
//! this is treated as mentioning every module-level name: each one it could
//! read lives in, or is mirrored into, the namespace dictionary like any
//! global fallback code names (`cascade`); and a unit whose code may also
//! bind one (`exec`, `globals()`, module-level `locals()`/`vars()`) is
//! treated as declaring every module variable `global`, so each lives in
//! the namespace dictionary, where compiled code reads what it binds.
//! Code reaching the module object (`sys.modules[__name__]`, `import
//! __main__`) may read and bind any of its attributes the same way.

use super::*;
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::Expr;

/// Builtins through which code reaches any module global by a string
/// wherever it calls them (`eval("c")`, `exec(src)`, `globals()["c"]`).
const GLOBALS_BY_STRING: &[&str] = &["globals", "eval", "exec"];

/// Builtins that read the namespace of the scope calling them when called
/// with no argument: at module level, the module's globals (`locals()`,
/// `vars()`, `dir()`).
const SCOPE_BY_STRING: &[&str] = &["locals", "vars", "dir"];

/// Of those, the ones whose code may also bind a global: `eval` evaluates
/// an expression, `dir()` lists names.
const BINDS_BY_STRING: &[&str] = &["globals", "exec", "locals", "vars"];

/// How a unit reaches module globals by a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Reach {
    /// It does not.
    No,
    /// It may read any module global.
    Reads,
    /// It may read and bind any module global.
    Binds,
}

/// How `mentions` reach module globals by a string through a builtin of
/// `GLOBALS_BY_STRING`, from any scope.
fn builtin_reach(mentions: &BTreeSet<String>) -> Reach {
    let names = GLOBALS_BY_STRING.iter().filter(|name| mentions.contains(**name));
    names.fold(Reach::No, |reach, name| reach.max(reach_of(name)))
}

/// Whether a nested `def` or `lambda` (`node`), reading the module globals
/// `mentions`, reaches module globals by a string: through a builtin of
/// `GLOBALS_BY_STRING`, or the module object (`ModuleObject`).
pub(super) fn nested_reaches(mentions: &BTreeSet<String>, node: NestedNode<'_>) -> bool {
    if builtin_reach(mentions) != Reach::No {
        return true;
    }
    let mut object = ModuleObject::default();
    match node {
        NestedNode::Def(def) => object.visit_stmt_function_def(def),
        NestedNode::Lambda(expr) => object.visit_expr(expr),
    }
    object.0
}

/// A nested statement, for `nested_reaches`.
pub(super) enum NestedNode<'a> {
    Def(&'a pycc_ast::StmtFunctionDef),
    Lambda(&'a Expr),
}

fn reach_of(name: &str) -> Reach {
    if BINDS_BY_STRING.contains(&name) {
        Reach::Binds
    } else {
        Reach::Reads
    }
}

/// How `stmt`, whose module-global references are `mentions`
/// (`names::module_references`), reaches module globals by a string: it
/// reads one of `GLOBALS_BY_STRING` as the builtin (not a local shadowing
/// it), calls one of `SCOPE_BY_STRING` with no argument at module scope
/// (outside every function, lambda and class body), or reaches the module
/// object (`ModuleObject`), which may bind any global too.
pub(super) fn reach(stmt: &Stmt, mentions: &BTreeSet<String>) -> Reach {
    let mut reach = builtin_reach(mentions);
    if reach == Reach::Binds {
        return reach;
    }
    let mut object = ModuleObject::default();
    object.visit_stmt(stmt);
    if object.0 {
        return Reach::Binds;
    }
    if !SCOPE_BY_STRING.iter().any(|name| mentions.contains(*name)) {
        return reach;
    }
    let mut finder = ModuleScope::default();
    finder.visit_stmt(stmt);
    for name in finder.0 {
        reach = reach.max(reach_of(name));
    }
    reach
}

/// Finds a no-argument call of a `SCOPE_BY_STRING` builtin at module scope.
#[derive(Default)]
struct ModuleScope(Vec<&'static str>);

impl<'a> Visitor<'a> for ModuleScope {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            // Only a definition's header runs at module scope.
            Stmt::FunctionDef(def) => {
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                for param in def.parameters.iter() {
                    if let Some(default) = param.default() {
                        self.visit_expr(default);
                    }
                }
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

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Lambda(lambda) => {
                if let Some(parameters) = &lambda.parameters {
                    for param in parameters.iter() {
                        if let Some(default) = param.default() {
                            self.visit_expr(default);
                        }
                    }
                }
            }
            Expr::Call(call) if call.arguments.is_empty() => {
                if let Expr::Name(name) = &*call.func
                    && let Some(found) = SCOPE_BY_STRING.iter().find(|n| **n == name.id.as_str())
                {
                    self.0.push(found);
                }
                visitor::walk_expr(self, expr);
            }
            _ => visitor::walk_expr(self, expr),
        }
    }

    fn visit_annotation(&mut self, _expr: &'a Expr) {}
}

/// Finds code reaching the module object, at any depth: an import of
/// `__main__` or a read of the name it binds, a subscript of a `modules`
/// mapping (`sys.modules`) by `__name__` or `"__main__"`, and a call of
/// `__import__` or an `import_module` with one of those first. Other uses of
/// `__name__` (`logging.getLogger(__name__)`, `if __name__ == "__main__":`)
/// do not reach the module.
#[derive(Default)]
struct ModuleObject(bool);

impl ModuleObject {
    fn visit_stmt_function_def(&mut self, def: &pycc_ast::StmtFunctionDef) {
        for decorator in &def.decorator_list {
            self.visit_decorator(decorator);
        }
        for param in def.parameters.iter() {
            if let Some(default) = param.default() {
                self.visit_expr(default);
            }
        }
        self.visit_body(&def.body);
    }
}

/// Whether `expr` names the running module: `__name__` or `"__main__"`.
fn names_module(expr: &Expr) -> bool {
    match expr {
        Expr::Name(name) => name.id.as_str() == "__name__",
        Expr::StringLiteral(literal) => literal.value.to_str() == "__main__",
        _ => false,
    }
}

impl<'a> Visitor<'a> for ModuleObject {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Import(import) if import.names.iter().any(|a| a.name.as_str() == "__main__") => {
                self.0 = true;
            }
            Stmt::ImportFrom(import)
                if import.level == 0
                    && import.module.as_ref().is_some_and(|m| m.as_str() == "__main__") =>
            {
                self.0 = true;
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) if name.id.as_str() == "__main__" => self.0 = true,
            Expr::Subscript(subscript)
                if names_module(&subscript.slice)
                    && matches!(&*subscript.value, Expr::Attribute(attr)
                        if attr.attr.as_str() == "modules") =>
            {
                self.0 = true;
            }
            Expr::Call(call)
                if call.arguments.args.first().is_some_and(names_module)
                    && match &*call.func {
                        Expr::Name(name) => {
                            matches!(name.id.as_str(), "__import__" | "import_module")
                        }
                        Expr::Attribute(attr) => attr.attr.as_str() == "import_module",
                        _ => false,
                    } =>
            {
                self.0 = true;
            }
            _ => visitor::walk_expr(self, expr),
        }
    }

    fn visit_annotation(&mut self, _expr: &'a Expr) {}
}

impl Plan {
    /// Makes every unit reaching module globals by a string (`reach`)
    /// mention every module-level name, and a unit that may bind one declare
    /// every module variable (module doc). Run once the units, lifts
    /// included, are known.
    pub(super) fn reach_by_string(&mut self) {
        let top = &self.stmts[..self.top];
        self.module_names = top
            .iter()
            .flat_map(|stmt| stmt.bound.iter())
            .filter(|name| !name.starts_with("__pycc") && name.as_str() != "*")
            .cloned()
            .collect();
        // The variables: names only plain statements bind. A `def`, class
        // or import name rebound by a string is not seen by compiled code
        // (`docs/DEVIATIONS.md`).
        let named: BTreeSet<&String> = top
            .iter()
            .filter(|stmt| !matches!(stmt.kind, Kind::Other))
            .flat_map(|stmt| stmt.bound.iter())
            .collect();
        let variables: BTreeSet<String> = top
            .iter()
            .filter(|stmt| matches!(stmt.kind, Kind::Other))
            .flat_map(|stmt| stmt.bound.iter())
            .filter(|name| self.module_names.contains(*name) && !named.contains(name))
            .cloned()
            .collect();
        for index in 0..self.stmts.len() {
            let reach = reach(&self.ast[index], &self.stmts[index].mentions);
            if reach >= Reach::Reads {
                let names = self.module_names.clone();
                self.stmts[index].mentions.extend(names);
            }
            if reach == Reach::Binds {
                self.stmts[index].declares.extend(variables.iter().cloned());
            }
        }
    }
}
