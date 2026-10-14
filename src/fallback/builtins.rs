//! Builtins compiled code reaches through CPython (#12): a use of a Python
//! builtin pycc does not compile is rewritten to go through the `builtins`
//! (or `sys`) module the fallback preamble imports, so the statement stays
//! compiled and only that one operation runs in CPython:
//!
//! - a builtin call pycc does not compile for its arguments
//!   (`min(ans, x)` on an object, `pow(a, b, m)`): `builtins.min(...)`;
//! - a builtin used as a value (`defaultdict(int)`, `l = len`):
//!   `builtins.int`;
//! - `exit(...)`/`quit(...)`, which `site` installs: `sys.exit(...)`;
//! - a method pycc does not compile on a `str`, `int` or `float` receiver
//!   (`"\n".join(objects)`, `"{}".format(x)`): `builtins.str.join("\n",
//!   objects)`.

use super::*;
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::Expr;

/// Builtins reading or writing their caller's frame, which a call through
/// the `builtins` module would get wrong.
const FRAME_BUILTINS: &[&str] = &[
    "super",
    "locals",
    "globals",
    "vars",
    "eval",
    "exec",
    "dir",
    "breakpoint",
];

/// The builtin values a bare name may be rewritten to, when pycc reports
/// the name undefined.
const VALUE_BUILTINS: &[&str] = &[
    "abs", "all", "any", "bin", "bool", "chr", "dict", "divmod", "enumerate", "filter", "float",
    "frozenset", "hash", "hex", "id", "input", "int", "isinstance", "iter", "len", "list", "map",
    "max", "min", "next", "oct", "open", "ord", "pow", "print", "range", "repr", "reversed",
    "round", "set", "sorted", "str", "sum", "tuple", "type", "zip",
];

impl Plan {
    /// Records the rewrite of the builtin use `failure` reports, returning
    /// whether it did (a use already rewritten, or none of the shapes
    /// above, is left to fall back).
    pub(super) fn rewrite_builtin_use(
        &mut self,
        rendered: Option<&Rendered>,
        failure: &Failure,
    ) -> bool {
        let Some(span) = failure.diagnostic.span.filter(|s| (s.start, s.end) != (0, 0)) else {
            return false;
        };
        let at = match rendered {
            Some(rendered) => rendered.source_offset(span.start),
            None => Some(span.start),
        };
        let Some(at) = at else {
            return false;
        };
        let message = failure.diagnostic.message.as_str();
        let edits = if let Some(name) = between(message, "call to builtin `", "` is valid Python")
        {
            // The span may cover an enclosing expression (`xs.append(chr(n))`
            // in a nested function): the first call of `name` inside it that
            // is not rewritten yet.
            let end = at + (span.end - span.start);
            (at..end.max(at + 1))
                .map(|start| self.builtin_call(start, name))
                .find(|edits| {
                    edits
                        .first()
                        .is_some_and(|(start, ..)| !self.object_builtins.contains_key(start))
                })
                .unwrap_or_default()
        } else if let Some(name) = between(message, "name `", "` is not defined") {
            self.builtin_value(at, name)
        } else if let Some(name) = between(message, "call to undefined function `", "`") {
            match name {
                "exit" | "quit" if self.name_at(at, name, true) => {
                    vec![(at, at + name.len() as u32, format!("{SYS}.exit"))]
                }
                _ => Vec::new(),
            }
        } else if let Some(ty) = between(message, "cannot call a method on `", "`: it is not") {
            match ty {
                "str" | "int" | "float" => self.method_through_type(at, ty),
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        if edits.is_empty() || edits.iter().any(|(start, ..)| self.object_builtins.contains_key(start)) {
            return false;
        }
        for (start, end, text) in edits {
            self.object_builtins.insert(start, (end, text));
        }
        true
    }

    fn builtin_call(&self, at: u32, name: &str) -> Vec<(u32, u32, String)> {
        // A builtin function, or a builtin exception class constructed
        // with a CPython object (`ValueError("bad: " + obj)`).
        let exception = ["Error", "Exception", "Warning"]
            .iter()
            .any(|suffix| name.ends_with(suffix));
        if !(name.starts_with(|c: char| c.is_ascii_lowercase()) || exception)
            || FRAME_BUILTINS.contains(&name)
            || !self.name_at(at, name, true)
        {
            return Vec::new();
        }
        vec![(at, at + name.len() as u32, format!("{BUILTINS}.{name}"))]
    }

    fn builtin_value(&self, at: u32, name: &str) -> Vec<(u32, u32, String)> {
        if !VALUE_BUILTINS.contains(&name)
            || !self.name_at(at, name, false)
            || self.stmts.iter().any(|stmt| stmt.bound.contains(name))
        {
            return Vec::new();
        }
        vec![(at, at + name.len() as u32, format!("{BUILTINS}.{name}"))]
    }

    /// Whether the source at `at` is the identifier `name` (followed by a
    /// call's `(` when `call`).
    fn name_at(&self, at: u32, name: &str, call: bool) -> bool {
        let Some(after) = self.source.get(at as usize..).and_then(|s| s.strip_prefix(name)) else {
            return false;
        };
        let ends = !after.starts_with(|c: char| c.is_alphanumeric() || c == '_');
        let starts = !self.source[..at as usize]
            .ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.');
        ends && starts && (!call || after.trim_start().starts_with('('))
    }

    /// `recv.method(args)` at `at` as `builtins.<ty>.method(recv, args)`.
    fn method_through_type(&self, at: u32, ty: &str) -> Vec<(u32, u32, String)> {
        let Some(index) = self.stmts.iter().position(|s| s.range.contains(&at)) else {
            return Vec::new();
        };
        let mut finder = MethodCall { at, found: None };
        finder.visit_stmt(&self.ast[index]);
        let Some((start, receiver_end, name_start, method, args_open, has_args)) = finder.found
        else {
            return Vec::new();
        };
        let Some(dot) = self
            .source
            .get(receiver_end as usize..name_start as usize)
            .and_then(|between| between.rfind('.'))
            .map(|offset| receiver_end + offset as u32)
        else {
            return Vec::new();
        };
        vec![
            (start, start, format!("{BUILTINS}.{ty}.{method}(")),
            (dot, args_open + 1, if has_args { ", " } else { "" }.to_string()),
        ]
    }
}

/// The text of `message` between `prefix` and the next `suffix`.
fn between<'m>(message: &'m str, prefix: &str, suffix: &str) -> Option<&'m str> {
    let rest = message.strip_prefix(prefix)?;
    Some(&rest[..rest.find(suffix)?])
}

/// Finds the method call `recv.m(args)` starting at `at`: its start, where
/// its receiver ends and its method name starts, the method name, the
/// offset of its `(`, and whether it has arguments.
struct MethodCall {
    at: u32,
    found: Option<(u32, u32, u32, String, u32, bool)>,
}

impl<'a> Visitor<'a> for MethodCall {
    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Expr::Call(call) = expr
            && self.found.is_none()
            && call.range.start().to_u32() == self.at
            && let Expr::Attribute(attr) = &*call.func
        {
            let receiver_end = pycc_ast::expr_range(&attr.value).end;
            let name_start = attr.attr.range.start().to_u32();
            let args_open = call.arguments.range.start().to_u32();
            self.found = Some((
                self.at,
                receiver_end,
                name_start,
                attr.attr.as_str().to_string(),
                args_open,
                !call.arguments.args.is_empty() || !call.arguments.keywords.is_empty(),
            ));
            return;
        }
        visitor::walk_expr(self, expr);
    }
}
