//! Guarded native specializations of an `--ext --non-strict` module's
//! unannotated functions and methods.
//!
//! Python may call an exported function with anything, so an unannotated
//! parameter of one is a CPython object, and every operation on it goes
//! through the C API. Most untyped code is called with one type per
//! parameter, though, and its body says which: a parameter that is indexed,
//! sliced, measured with `len()` or given a `str` method is text; one used
//! as an index, a slice bound or a `range()` argument, or in arithmetic with
//! an integer literal, is an `int`. For such a function this pass adds a
//! *native specialization*, a copy of the `def` with those parameters
//! annotated, and puts a guard at the top of the original:
//!
//! ```python
//! def diff_bisect(self, text1, text2, deadline):
//!     if type(text1) is builtins.str and type(text2) is builtins.str:
//!         return self._pycc_native_diff_bisect(text1, text2, deadline)
//!     ...                                     # the object path, unchanged
//! def _pycc_native_diff_bisect(self, text1: str, text2: str, deadline: object) -> object:
//!     ...                                     # the same body, native
//! ```
//!
//! The guard tests the *exact* type, so a subclass of `str` (which may
//! override any operation) keeps the object path, and the specialization
//! runs only on the values its annotations describe: a guess that is wrong
//! costs a failed test per call, never a different result. Only immutable
//! values are guessed (`str`, `int`): converting a CPython `list` into a
//! native one would copy it, and the caller would no longer see the
//! function's stores into it.
//!
//! The copy keeps the source ranges of the original, so a traceback through
//! it names the original's lines. A specialization the checker refuses is
//! dropped by the driver (`skip`), which then re-runs this pass without it;
//! [`Specialized`] records what each one covers for that attribution.
//!
//! Only a function that loops (a `for`, a `while` or a comprehension) is
//! specialized: converting a `str` argument copies it, which a body that
//! does constant work would not win back.

use super::build::name;
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    CmpOp, Expr, ExprContext, Identifier, ModModule, Number, Operator, Stmt, StmtFunctionDef,
};
use ruff_text_size::{Ranged, TextRange};
use std::collections::{BTreeMap, HashSet};

/// The prefix of a specialization's name.
pub const NATIVE_PREFIX: &str = "_pycc_native_";

/// The module-level name the guards read CPython's `builtins` module
/// through: `type(x) is str` on a CPython object compares with the `str`
/// type object, which compiled code reaches as an attribute of the module.
const BUILTINS: &str = "__pycc_specialize_builtins";

/// One planned specialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Specialized {
    /// `f`, or `Class.method`.
    pub qualname: String,
    /// The original `def`'s range, which its copy shares.
    pub range: TextRange,
    /// The guessed type of each specialized parameter.
    pub params: Vec<(String, &'static str)>,
}

/// Adds the guarded specializations of `module`'s unannotated top-level
/// functions and methods, except those `skip` names, and returns them.
pub fn specialize_untyped(module: &mut ModModule, skip: &dyn Fn(&str) -> bool) -> Vec<Specialized> {
    // The guard spells `type`: a module that binds it (or the names the
    // guard's types are read as) anywhere is left alone.
    if ["type", "str", "int", "builtins"]
        .iter()
        .any(|builtin| super::ext_generators::bindings(&module.body, builtin) > 0)
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    let old = std::mem::take(&mut module.body);
    let mut body = Vec::with_capacity(old.len());
    for stmt in old {
        match stmt {
            Stmt::FunctionDef(mut def) => {
                let planned = plan(&def, None, skip);
                match planned {
                    Some((clone, spec)) => {
                        add_guard(&mut def, None, &spec);
                        body.push(Stmt::FunctionDef(def));
                        body.push(Stmt::FunctionDef(clone));
                        out.push(spec);
                    }
                    None => body.push(Stmt::FunctionDef(def)),
                }
            }
            Stmt::ClassDef(mut class) => {
                let class_name = class.name.to_string();
                let old = std::mem::take(&mut class.body);
                let mut members = Vec::with_capacity(old.len());
                for item in old {
                    match item {
                        Stmt::FunctionDef(mut def) => {
                            match plan(&def, Some(&class_name), skip) {
                                Some((clone, spec)) => {
                                    add_guard(&mut def, Some(&class_name), &spec);
                                    members.push(Stmt::FunctionDef(def));
                                    members.push(Stmt::FunctionDef(clone));
                                    out.push(spec);
                                }
                                None => members.push(Stmt::FunctionDef(def)),
                            }
                        }
                        other => members.push(other),
                    }
                }
                class.body = members.into();
                body.push(Stmt::ClassDef(class));
            }
            other => body.push(other),
        }
    }
    if !out.is_empty() {
        // After the docstring and the `__future__` imports.
        let mut at = usize::from(matches!(body.first(), Some(Stmt::Expr(e))
            if e.value.is_string_literal_expr()));
        while matches!(body.get(at), Some(Stmt::ImportFrom(import))
            if import.module.as_ref().map(|m| m.as_str()) == Some("__future__"))
        {
            at += 1;
        }
        let import = super::signatures::parse_helper(
            &format!("import builtins as {BUILTINS}\n"),
            TextRange::default(),
        );
        for (offset, stmt) in import.into_iter().enumerate() {
            body.insert(at + offset, stmt);
        }
    }
    module.body = body.into();
    out
}

/// The type an unannotated parameter with literal default `default` has
/// (as the original's lowering gives it, #1409).
fn literal_type(default: &Expr) -> Option<&'static str> {
    Some(match default {
        Expr::NumberLiteral(n) => match n.value {
            Number::Int(_) => "int",
            Number::Float(_) => "float",
            Number::Complex { .. } => return None,
        },
        Expr::BooleanLiteral(_) => "bool",
        Expr::StringLiteral(_) => "str",
        Expr::NoneLiteral(_) => "object",
        Expr::UnaryOp(u) => return literal_type(&u.operand),
        _ => return None,
    })
}

/// The specialization of `def` (a method of `class` when given) and what it
/// covers, or `None` when it is not specialized.
fn plan(
    def: &StmtFunctionDef,
    class: Option<&str>,
    skip: &dyn Fn(&str) -> bool,
) -> Option<(StmtFunctionDef, Specialized)> {
    let fn_name = def.name.as_str();
    let p = &def.parameters;
    if fn_name.starts_with('_')
        || def.is_async
        || !def.decorator_list.is_empty()
        || p.vararg.is_some()
        || p.kwarg.is_some()
        || !p.posonlyargs.is_empty()
        || !p.kwonlyargs.is_empty()
        || super::ext_generators::yields(&def.body)
        || !loops(&def.body)
    {
        return None;
    }
    let qualname = match class {
        Some(class) => format!("{class}.{fn_name}"),
        None => fn_name.to_string(),
    };
    if skip(&qualname) {
        return None;
    }
    let (receiver, params) = match class {
        Some(_) => {
            let (first, rest) = p.args.split_first()?;
            if first.default.is_some() {
                return None;
            }
            (Some(first), rest)
        }
        None => (None, &p.args[..]),
    };
    let names: Vec<String> = params.iter().map(|a| a.parameter.name.to_string()).collect();
    if names.iter().any(|n| n.starts_with("_pycc")) {
        return None;
    }
    let evidence = Evidence::of(&def.body, &names);
    let mut guessed = Vec::new();
    let mut annotations = Vec::with_capacity(params.len());
    for param in params {
        let param_name = param.parameter.name.as_str();
        let annotation: Expr = match (&param.parameter.annotation, &param.default) {
            (Some(annotation), _) => annotation.as_ref().clone(),
            (None, Some(default)) => {
                if !super::signatures::is_literal(default) {
                    return None;
                }
                name(literal_type(default)?, ExprContext::Load, param.range())
            }
            (None, None) => {
                let ty = evidence.guess(param_name).unwrap_or("object");
                if ty != "object" {
                    guessed.push((param_name.to_string(), ty));
                }
                name(ty, ExprContext::Load, param.range())
            }
        };
        annotations.push(annotation);
    }
    if guessed.is_empty() {
        return None;
    }
    let mut clone = def.clone();
    clone.name = Identifier::new(format!("{NATIVE_PREFIX}{fn_name}"), def.name.range());
    let offset = usize::from(receiver.is_some());
    for (index, annotation) in annotations.into_iter().enumerate() {
        let arg = &mut clone.parameters.args[index + offset];
        arg.default = None;
        arg.parameter.annotation = Some(Box::new(annotation));
    }
    if let Some(first) = clone.parameters.args.first_mut()
        && receiver.is_some()
    {
        first.default = None;
    }
    if clone.returns.is_none() {
        clone.returns = Some(Box::new(name("object", ExprContext::Load, def.name.range())));
    }
    Some((
        clone,
        Specialized {
            qualname,
            range: def.range(),
            params: guessed,
        },
    ))
}

/// Puts the guard calling `spec`'s specialization at the top of `def`
/// (after its docstring).
fn add_guard(def: &mut StmtFunctionDef, class: Option<&str>, spec: &Specialized) {
    let names: Vec<String> = def
        .parameters
        .args
        .iter()
        .map(|a| a.parameter.name.to_string())
        .collect();
    let (callee, args) = match class {
        Some(_) => (
            format!("{}.{NATIVE_PREFIX}{}", names[0], def.name),
            names[1..].join(", "),
        ),
        None => (format!("{NATIVE_PREFIX}{}", def.name), names.join(", ")),
    };
    let test = spec
        .params
        .iter()
        .map(|(param, ty)| format!("type({param}) is {BUILTINS}.{ty}"))
        .collect::<Vec<_>>()
        .join(" and ");
    let source = format!("if {test}:\n    return {callee}({args})\n");
    let guard = super::signatures::parse_helper(&source, def.name.range());
    let at = usize::from(matches!(def.body.first(), Some(Stmt::Expr(e))
        if matches!(e.value.as_ref(), Expr::StringLiteral(_))));
    for (offset, stmt) in guard.into_iter().enumerate() {
        def.body.insert(at + offset, stmt);
    }
}

/// Whether `body` loops (not counting nested functions and classes).
fn loops(body: &[Stmt]) -> bool {
    struct Find(bool);
    impl<'a> Visitor<'a> for Find {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
                Stmt::For(_) | Stmt::While(_) => self.0 = true,
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Lambda(_) => {}
                Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {
                    self.0 = true;
                }
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    let mut find = Find(false);
    for stmt in body {
        find.visit_stmt(stmt);
    }
    find.0
}

/// `str` methods a `list`, `dict`, `set` or `tuple` does not have.
const STR_METHODS: &[&str] = &[
    "find", "rfind", "rindex", "startswith", "endswith", "split", "rsplit", "splitlines",
    "strip", "lstrip", "rstrip", "lower", "upper", "replace", "join", "isdigit", "isalpha",
    "isalnum", "isspace", "isupper", "islower", "encode", "partition", "rpartition",
    "zfill", "ljust", "rjust", "center", "casefold", "title", "capitalize", "swapcase",
    "expandtabs", "format", "removeprefix", "removesuffix",
];

/// Methods of a mutable container, which a `str` does not have.
const CONTAINER_METHODS: &[&str] = &[
    "append", "extend", "insert", "pop", "remove", "sort", "reverse", "clear", "add",
    "update", "keys", "values", "items", "get", "setdefault", "discard", "copy",
    "popitem", "difference", "union", "intersection",
];

#[derive(Default, Clone, Copy)]
struct Uses {
    /// A `str`-only use: a `str` method, `+`/`==` with a string literal.
    text: bool,
    /// A sequence use: indexed, sliced, `len()`.
    sequence: bool,
    /// A use a `str` cannot have: a container method, an item store, an
    /// item read with a string key, an item of an item.
    not_text: bool,
    /// An `int` use: an index or slice bound, a `range()` argument,
    /// arithmetic or an order comparison with an integer literal.
    integer: bool,
    /// A use an `int` cannot have (or should not be guessed from): an
    /// attribute, a float literal operand.
    not_integer: bool,
}

/// How a function body uses each of its parameters.
struct Evidence {
    uses: BTreeMap<String, Uses>,
}

impl Evidence {
    fn of(body: &[Stmt], params: &[String]) -> Self {
        let mut walker = Walker {
            params: params.iter().cloned().collect(),
            uses: BTreeMap::new(),
        };
        for stmt in body {
            walker.visit_stmt(stmt);
        }
        Self { uses: walker.uses }
    }

    /// The type `param` is guessed to have, if any.
    fn guess(&self, param: &str) -> Option<&'static str> {
        let uses = self.uses.get(param).copied().unwrap_or_default();
        if (uses.text || uses.sequence) && !uses.not_text {
            Some("str")
        } else if uses.integer && !uses.not_integer && !uses.sequence && !uses.text {
            Some("int")
        } else {
            None
        }
    }
}

struct Walker {
    params: HashSet<String>,
    uses: BTreeMap<String, Uses>,
}

impl Walker {
    fn param<'e>(&self, expr: &'e Expr) -> Option<&'e str> {
        match expr {
            Expr::Name(n) if self.params.contains(n.id.as_str()) => Some(n.id.as_str()),
            _ => None,
        }
    }

    fn mark(&mut self, expr: &Expr, f: impl FnOnce(&mut Uses)) {
        if let Some(param) = self.param(expr) {
            f(self.uses.entry(param.to_string()).or_default());
        }
    }

    /// Marks the names an index or slice bound reads directly.
    fn mark_index(&mut self, index: &Expr) {
        match index {
            Expr::Slice(slice) => {
                for bound in [&slice.lower, &slice.upper, &slice.step].into_iter().flatten() {
                    self.mark_index(bound);
                }
            }
            Expr::UnaryOp(u) => self.mark_index(&u.operand),
            Expr::BinOp(b) => {
                self.mark_index(&b.left);
                self.mark_index(&b.right);
            }
            other => self.mark(other, |u| u.integer = true),
        }
    }

    fn literal_operand(&mut self, operand: &Expr, other: &Expr) {
        match other {
            Expr::StringLiteral(_) | Expr::FString(_) => self.mark(operand, |u| u.text = true),
            Expr::NumberLiteral(n) => match n.value {
                Number::Int(_) => self.mark(operand, |u| u.integer = true),
                _ => self.mark(operand, |u| u.not_integer = true),
            },
            _ => {}
        }
    }
}

impl<'a> Visitor<'a> for Walker {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            // A nested scope's names are its own.
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
            Stmt::AugAssign(aug) => {
                self.literal_operand(&aug.target, &aug.value);
                visitor::walk_stmt(self, stmt);
            }
            Stmt::Delete(del) => {
                for target in &del.targets {
                    if let Expr::Subscript(s) = target {
                        self.mark(&s.value, |u| u.not_text = true);
                    }
                }
                visitor::walk_stmt(self, stmt);
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Lambda(_) => return,
            Expr::Call(call) => {
                if let Expr::Attribute(attr) = call.func.as_ref() {
                    let method = attr.attr.as_str();
                    if STR_METHODS.contains(&method) {
                        self.mark(&attr.value, |u| u.text = true);
                    } else if CONTAINER_METHODS.contains(&method) {
                        self.mark(&attr.value, |u| u.not_text = true);
                    }
                }
                if let Expr::Name(f) = call.func.as_ref() {
                    match f.id.as_str() {
                        "len" => {
                            if let [arg] = &call.arguments.args[..] {
                                self.mark(arg, |u| u.sequence = true);
                            }
                        }
                        "range" => {
                            for arg in &call.arguments.args {
                                self.mark_index(arg);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Expr::Attribute(attr) => {
                // A `str` has methods, but no attribute worth reading or
                // storing.
                let text_method = STR_METHODS.contains(&attr.attr.as_str())
                    || matches!(attr.attr.as_str(), "count" | "index");
                self.mark(&attr.value, |u| {
                    u.not_integer = true;
                    u.not_text |= !text_method;
                });
            }
            Expr::Subscript(sub) => {
                self.mark(&sub.value, |u| {
                    u.sequence = true;
                    u.not_integer = true;
                });
                if sub.ctx != ExprContext::Load {
                    self.mark(&sub.value, |u| u.not_text = true);
                }
                if matches!(sub.slice.as_ref(), Expr::StringLiteral(_)) {
                    self.mark(&sub.value, |u| u.not_text = true);
                }
                if let Expr::Subscript(inner) = sub.value.as_ref() {
                    self.mark(&inner.value, |u| u.not_text = true);
                }
                self.mark_index(&sub.slice);
            }
            Expr::BinOp(bin) => {
                if matches!(
                    bin.op,
                    Operator::Add
                        | Operator::Sub
                        | Operator::Mult
                        | Operator::FloorDiv
                        | Operator::Mod
                        | Operator::BitAnd
                        | Operator::BitOr
                        | Operator::BitXor
                        | Operator::LShift
                        | Operator::RShift
                ) {
                    self.literal_operand(&bin.left, &bin.right);
                    self.literal_operand(&bin.right, &bin.left);
                }
            }
            Expr::Compare(cmp) => {
                let mut left = cmp.left.as_ref();
                for (op, right) in cmp.ops.iter().zip(&cmp.comparators) {
                    if matches!(
                        op,
                        CmpOp::Lt | CmpOp::LtE | CmpOp::Gt | CmpOp::GtE | CmpOp::Eq | CmpOp::NotEq
                    ) {
                        self.literal_operand(left, right);
                        self.literal_operand(right, left);
                    }
                    left = right;
                }
            }
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }
}

#[cfg(test)]
mod tests;
