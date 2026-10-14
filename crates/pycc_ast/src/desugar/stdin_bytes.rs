//! When `sys.stdin.buffer.readline` may be read as text.
//!
//! `sys.stdin.buffer.readline()` returns `bytes`, which prints, indexes,
//! iterates and compares unlike `str`, so it is not the native `str`
//! readline in general. One use is common enough to matter and is
//! indistinguishable from text: a module-level alias
//! (`input = sys.stdin.buffer.readline`, or a `def` whose body is `return
//! sys.stdin.buffer.readline()`) whose every call is converted to
//! numbers at once -- `int(input())`, `float(input())`,
//! `map(int, input().split())` and `[int(x) for x in input().split()]`.
//! `int()` and `float()` accept the same ASCII digits, signs and
//! surrounding whitespace from `bytes` as from `str`, and `split()` without
//! a separator splits both on that whitespace, so for such a module
//! [`buffer_reads_as_text`] lets the alias be the native text readline.

use super::aliases::{SysNames, rebound_names};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, Stmt};
use ruff_text_size::{Ranged, TextRange};
use std::collections::HashSet;

/// Whether every read of `sys.stdin.buffer` in `body` is the value of a
/// module-level alias `name = <stdin>.buffer.readline` (bound once) whose
/// every use is a call converted to numbers (module doc).
pub(crate) fn buffer_reads_as_text(body: &[Stmt], sys: &SysNames) -> bool {
    let rebound = rebound_names(body);
    if ["int", "float", "map"]
        .iter()
        .any(|name| rebound.contains_key(*name))
    {
        return false;
    }
    let mut aliases: HashSet<String> = HashSet::new();
    let mut alias_values: HashSet<TextRange> = HashSet::new();
    for stmt in body {
        // `def _r(): return sys.stdin.buffer.readline()` is the same alias.
        if let Stmt::FunctionDef(def) = stmt
            && def.decorator_list.is_empty()
            && def.parameters.is_empty()
            && rebound.get(def.name.as_str()).copied() == Some(1)
            && let [Stmt::Return(ret)] = &def.body[..]
            && let Some(Expr::Call(call)) = ret.value.as_deref()
            && call.arguments.is_empty()
            && let Expr::Attribute(readline) = call.func.as_ref()
            && readline.attr.as_str() == "readline"
            && is_buffer(sys, &readline.value)
        {
            aliases.insert(def.name.to_string());
            alias_values.insert(readline.value.range());
            continue;
        }
        let Stmt::Assign(assign) = stmt else { continue };
        let [Expr::Name(target)] = assign.targets.as_slice() else {
            continue;
        };
        if let Expr::Attribute(readline) = assign.value.as_ref()
            && readline.attr.as_str() == "readline"
            && is_buffer(sys, &readline.value)
            && rebound.get(target.id.as_str()).copied() == Some(1)
        {
            aliases.insert(target.id.to_string());
            alias_values.insert(readline.value.range());
        }
    }
    if aliases.is_empty() {
        return false;
    }
    let mut scan = Scan {
        sys,
        aliases: &aliases,
        alias_values: &alias_values,
        safe: HashSet::new(),
        uses: Vec::new(),
        stray_buffer: false,
    };
    for stmt in body {
        scan.visit_stmt(stmt);
    }
    !scan.stray_buffer && scan.uses.iter().all(|range| scan.safe.contains(range))
}

/// `<stdin>.buffer`.
fn is_buffer(sys: &SysNames, expr: &Expr) -> bool {
    matches!(expr, Expr::Attribute(a) if a.attr.as_str() == "buffer" && sys.is_stdin(&a.value))
}

struct Scan<'s> {
    sys: &'s SysNames,
    aliases: &'s HashSet<String>,
    alias_values: &'s HashSet<TextRange>,
    /// The ranges of alias names in a numeric-conversion position.
    safe: HashSet<TextRange>,
    /// The ranges of every read of an alias name.
    uses: Vec<TextRange>,
    stray_buffer: bool,
}

impl Scan<'_> {
    /// The alias name `expr` calls with no arguments (`input()`).
    fn alias_call(&self, expr: &Expr) -> Option<TextRange> {
        let Expr::Call(call) = expr else { return None };
        let Expr::Name(name) = call.func.as_ref() else {
            return None;
        };
        (self.aliases.contains(name.id.as_str()) && call.arguments.is_empty()).then(|| name.range)
    }

    /// The alias call `expr` splits without a separator (`input().split()`).
    fn split_alias_call(&self, expr: &Expr) -> Option<TextRange> {
        let Expr::Call(call) = expr else { return None };
        let Expr::Attribute(split) = call.func.as_ref() else {
            return None;
        };
        if split.attr.as_str() != "split" || !call.arguments.is_empty() {
            return None;
        }
        self.alias_call(&split.value)
    }

    /// Records the alias reads `expr` converts to numbers at once.
    fn mark_safe(&mut self, expr: &Expr) {
        match expr {
            // `int(input())`, `float(input())`
            Expr::Call(call)
                if is_number_builtin(&call.func) && call.arguments.keywords.is_empty() =>
            {
                if let [arg] = &call.arguments.args[..]
                    && let Some(range) = self.alias_call(arg)
                {
                    self.safe.insert(range);
                }
            }
            _ => {}
        }
        match expr {
            // `map(int, input().split())`
            Expr::Call(call)
                if matches!(call.func.as_ref(), Expr::Name(n) if n.id.as_str() == "map")
                    && call.arguments.keywords.is_empty() =>
            {
                if let [function, iterable] = &call.arguments.args[..]
                    && is_number_builtin(function)
                    && let Some(range) = self.split_alias_call(iterable)
                {
                    self.safe.insert(range);
                }
            }
            // `[int(x) for x in input().split()]`
            Expr::ListComp(comp) => {
                if let [generator] = &comp.generators[..]
                    && generator.ifs.is_empty()
                    && !generator.is_async
                    && let Expr::Name(var) = &generator.target
                    && let Expr::Call(convert) = comp.elt.as_ref()
                    && is_number_builtin(&convert.func)
                    && convert.arguments.keywords.is_empty()
                    && matches!(&convert.arguments.args[..], [Expr::Name(arg)] if arg.id == var.id)
                    && let Some(range) = self.split_alias_call(&generator.iter)
                {
                    self.safe.insert(range);
                }
            }
            _ => {}
        }
    }
}

/// `int` or `float`, the builtins that read `bytes` digits as `str` ones.
fn is_number_builtin(expr: &Expr) -> bool {
    matches!(expr, Expr::Name(n) if matches!(n.id.as_str(), "int" | "float"))
}

impl<'a> Visitor<'a> for Scan<'_> {
    fn visit_expr(&mut self, expr: &'a Expr) {
        self.mark_safe(expr);
        match expr {
            Expr::Name(name) if self.aliases.contains(name.id.as_str()) => {
                if name.ctx == ruff_python_ast::ExprContext::Load {
                    self.uses.push(name.range);
                }
            }
            Expr::Attribute(attr)
                if attr.attr.as_str() == "buffer"
                    && self.sys.is_stdin(&attr.value)
                    && !self.alias_values.contains(&expr.range()) =>
            {
                self.stray_buffer = true;
            }
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }
}
