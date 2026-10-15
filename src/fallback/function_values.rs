//! Compiled functions used as values of differing types (#12).
//!
//! A name bound to one of two compiled functions with different signatures
//! (`parse = parse_a if multiline else parse_b`, where `parse_b` takes a
//! keyword-only flag `parse_a` does not) has no one native function type.
//! Python code sees every public compiled function as the callable the
//! module publishes, so the fallback loop reads those functions from the
//! module's namespace instead (`ns['parse_a']`): the name holds CPython
//! callables, the calls through it go through CPython, and everything else
//! in the function stays compiled.

use super::*;
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::Expr;

impl Plan {
    /// Records the namespace reads of the functions bound to the name a
    /// `T0023` between two function types names, returning whether it did.
    pub(super) fn functions_as_objects(
        &mut self,
        rendered: Option<&Rendered>,
        failure: &Failure,
    ) -> bool {
        let message = failure.diagnostic.message.as_str();
        if failure.diagnostic.code != "T0023"
            || !message.starts_with("cannot assign `Callable[")
            || !message.contains("previously inferred as `Callable[")
            || !self.publishes.functions
        {
            return false;
        }
        let Some(name) = message
            .split("` to `")
            .nth(1)
            .and_then(|rest| rest.split('`').next())
        else {
            return false;
        };
        let Some(index) = self.owner_of(rendered, failure) else {
            return false;
        };
        let published: BTreeSet<&str> = self
            .stmts
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.fallback.contains_key(i))
            .filter_map(|(_, stmt)| match &stmt.kind {
                Kind::Def(def, _) if !def.starts_with('_') => Some(def.as_str()),
                _ => None,
            })
            .collect();
        let mut finder = FunctionValues {
            target: name,
            published: &published,
            found: Vec::new(),
        };
        finder.visit_stmt(&self.ast[index]);
        let mut progress = false;
        for (range, function) in finder.found {
            if self.object_builtins.contains_key(&range.start) {
                continue;
            }
            self.object_builtins
                .insert(range.start, (range.end, format!("{NS}[{}]", py_str(&function))));
            progress = true;
        }
        progress
    }
}

/// The values `target = f` binds where `f` is a published function.
struct FunctionValues<'a> {
    target: &'a str,
    published: &'a BTreeSet<&'a str>,
    found: Vec<(Range<u32>, String)>,
}

impl<'a> Visitor<'a> for FunctionValues<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if let Stmt::Assign(assign) = stmt
            && let [Expr::Name(target)] = assign.targets.as_slice()
            && target.id.as_str() == self.target
            && let Expr::Name(value) = &*assign.value
            && self.published.contains(value.id.as_str())
        {
            self.found.push((
                pycc_ast::expr_range(&assign.value),
                value.id.as_str().to_string(),
            ));
        }
        visitor::walk_stmt(self, stmt);
    }
}
