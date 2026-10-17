//! What a function body says about the types of its parameters: the
//! evidence `specialize`'s guesses rest on (`super`'s module doc).

use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{CmpOp, Expr, ExprContext, Number, Operator, Stmt};
use ruff_text_size::Ranged;
use std::collections::{BTreeMap, BTreeSet, HashSet};

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
    /// An item used as a number (module doc): a byte buffer's use.
    byte_item: bool,
}

/// How a function body uses each of its parameters.
pub(super) struct Evidence {
    uses: BTreeMap<String, Uses>,
    /// The parameters the body binds again ([`Locals::rebound`]).
    rebound: HashSet<String>,
}

impl Evidence {
    pub(super) fn of(body: &[Stmt], params: &[String]) -> Self {
        let params: HashSet<String> = params.iter().cloned().collect();
        let (item_of, derived, rebound) = Locals::of(body, &params);
        let mut walker = Walker {
            params,
            item_of,
            derived,
            uses: BTreeMap::new(),
        };
        for stmt in body {
            walker.visit_stmt(stmt);
        }
        Self {
            uses: walker.uses,
            rebound,
        }
    }

    /// The type `param` is guessed to have, if any; `passed_bytearray`
    /// when every call of the function in the module passes a `bytearray`
    /// for it (`call_sites`), which outweighs the sequence evidence that
    /// would otherwise guess `str`.
    pub(super) fn guess(&self, param: &str, passed_bytearray: bool) -> Option<&'static str> {
        let uses = self.uses.get(param).copied().unwrap_or_default();
        // A byte buffer parameter is checked to be one again at each
        // binding, which a rebinding to anything else would fail.
        if (uses.byte_item || passed_bytearray) && !uses.text && !self.rebound.contains(param) {
            Some("bytearray")
        } else if (uses.text || uses.sequence) && !uses.not_text {
            Some("str")
        } else if uses.integer && !uses.not_integer && !uses.sequence && !uses.text {
            Some("int")
        } else {
            None
        }
    }
}

/// What the bindings of a function's locals say about its parameters.
struct Locals<'p> {
    params: &'p HashSet<String>,
    /// local -> the parameter every binding of it but an integer literal is
    /// an item of (`x = s[i]`): a number use of `x` is one of the item.
    items: BTreeMap<String, String>,
    /// local -> the parameters every binding of it computes it from by
    /// integer arithmetic (`i = -n`, `i += 1`): an index use of `i` is an
    /// `int` use of each of them.
    derived: BTreeMap<String, BTreeSet<String>>,
    /// Locals bound some other way too.
    not_item: HashSet<String>,
    not_derived: HashSet<String>,
    /// The parameters the body binds again, which are not guessed
    /// `bytearray` (`Evidence::guess`).
    rebound: HashSet<String>,
}

/// [`Locals::of`]'s findings: its `items`, `derived` and `rebound`.
type Bindings = (
    BTreeMap<String, String>,
    BTreeMap<String, BTreeSet<String>>,
    HashSet<String>,
);

impl Locals<'_> {
    fn of(body: &[Stmt], params: &HashSet<String>) -> Bindings {
        let mut locals = Locals {
            params,
            items: BTreeMap::new(),
            derived: BTreeMap::new(),
            not_item: HashSet::new(),
            not_derived: HashSet::new(),
            rebound: HashSet::new(),
        };
        for stmt in body {
            locals.visit_stmt(stmt);
        }
        let mut items = locals.items;
        items.retain(|local, _| !locals.not_item.contains(local));
        let mut derived = locals.derived;
        derived.retain(|local, _| !locals.not_derived.contains(local));
        (items, derived, locals.rebound)
    }

    /// The parameters integer arithmetic `value` computes from, or `None`
    /// when it is not such arithmetic.
    fn arithmetic(&self, value: &Expr) -> Option<BTreeSet<String>> {
        match value {
            Expr::NumberLiteral(n) if matches!(n.value, Number::Int(_)) => Some(BTreeSet::new()),
            Expr::Name(n) if self.params.contains(n.id.as_str()) => {
                Some(BTreeSet::from([n.id.to_string()]))
            }
            Expr::Name(n) => self.derived.get(n.id.as_str()).cloned(),
            Expr::UnaryOp(u)
                if matches!(
                    u.op,
                    ruff_python_ast::UnaryOp::USub
                        | ruff_python_ast::UnaryOp::UAdd
                        | ruff_python_ast::UnaryOp::Invert
                ) =>
            {
                self.arithmetic(&u.operand)
            }
            Expr::BinOp(bin) if bin.op != Operator::Div && bin.op != Operator::Pow => {
                let mut params = self.arithmetic(&bin.left)?;
                params.extend(self.arithmetic(&bin.right)?);
                Some(params)
            }
            _ => None,
        }
    }

    /// Records one binding of `target` to `value` (`None`: unknown), or an
    /// augmented one (`aug`).
    fn bind(&mut self, target: &Expr, value: Option<&Expr>, aug: bool) {
        match target {
            Expr::Name(n) => {
                let local = n.id.to_string();
                if self.params.contains(&local) {
                    self.rebound.insert(local.clone());
                }
                let literal = value.is_some_and(
                    |v| matches!(v, Expr::NumberLiteral(n) if matches!(n.value, Number::Int(_))),
                );
                let item_of = match value {
                    Some(Expr::Subscript(sub)) if !aug && !sub.slice.is_slice_expr() => {
                        match sub.value.as_ref() {
                            Expr::Name(base) if self.params.contains(base.id.as_str()) => {
                                Some(base.id.to_string())
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                match item_of {
                    Some(param) if self.items.get(&local).is_none_or(|p| *p == param) => {
                        self.items.insert(local.clone(), param);
                    }
                    None if literal && !aug => {}
                    _ => {
                        self.not_item.insert(local.clone());
                    }
                }
                match value.and_then(|v| self.arithmetic(v)) {
                    Some(params) => self.derived.entry(local).or_default().extend(params),
                    None => {
                        self.not_derived.insert(local);
                    }
                }
            }
            Expr::Tuple(t) => t.elts.iter().for_each(|e| self.bind(e, None, false)),
            Expr::List(l) => l.elts.iter().for_each(|e| self.bind(e, None, false)),
            Expr::Starred(s) => self.bind(&s.value, None, false),
            _ => {}
        }
    }
}

impl<'a> Visitor<'a> for Locals<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => return,
            Stmt::Assign(assign) => {
                for target in &assign.targets {
                    self.bind(target, Some(&assign.value), false);
                }
            }
            Stmt::AugAssign(aug) => self.bind(&aug.target, Some(&aug.value), true),
            Stmt::AnnAssign(ann) => self.bind(&ann.target, None, false),
            // `for x in s:` binds `x` to an item of `s`, as `x = s[i]` does.
            Stmt::For(f) => match (f.target.as_ref(), f.iter.as_ref()) {
                (Expr::Name(_), Expr::Name(base)) if self.params.contains(base.id.as_str()) => {
                    let item = Expr::Subscript(ruff_python_ast::ExprSubscript {
                        node_index: Default::default(),
                        range: f.iter.range(),
                        value: f.iter.clone(),
                        slice: Box::new(Expr::NumberLiteral(ruff_python_ast::ExprNumberLiteral {
                            node_index: Default::default(),
                            range: f.iter.range(),
                            value: Number::Int(ruff_python_ast::Int::ZERO),
                        })),
                        ctx: ExprContext::Load,
                    });
                    self.bind(&f.target, Some(&item), false);
                }
                _ => self.bind(&f.target, None, false),
            },
            Stmt::With(w) => {
                for item in &w.items {
                    if let Some(var) = &item.optional_vars {
                        self.bind(var, None, false);
                    }
                }
            }
            _ => {}
        }
        visitor::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Lambda(_) => return,
            Expr::Named(named) => self.bind(&named.target, None, false),
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }
}

struct Walker {
    params: HashSet<String>,
    /// [`Locals::items`]: local -> the parameter it is an item of.
    item_of: BTreeMap<String, String>,
    /// [`Locals::derived`]: local -> the parameters it is computed from.
    derived: BTreeMap<String, BTreeSet<String>>,
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
            Expr::Name(n) if self.derived.contains_key(n.id.as_str()) => {
                for param in self.derived[n.id.as_str()].clone() {
                    self.uses.entry(param).or_default().integer = true;
                }
            }
            other => self.mark(other, |u| u.integer = true),
        }
    }

    /// Marks the parameter whose item `expr` is (directly, or through an
    /// [`Locals::items`] local) as used as a number. `expr` is an operand of
    /// an operation only numbers have, so the operands of any arithmetic
    /// in it (a `+`, which `str` has too) are numbers as well.
    fn mark_number(&mut self, expr: &Expr) {
        match expr {
            Expr::Name(n) => {
                if let Some(param) = self.item_of.get(n.id.as_str()).cloned() {
                    self.uses.entry(param).or_default().byte_item = true;
                }
            }
            Expr::Subscript(sub) if !sub.slice.is_slice_expr() => {
                self.mark(&sub.value, |u| u.byte_item = true);
            }
            Expr::BinOp(bin) => {
                self.mark_number(&bin.left);
                self.mark_number(&bin.right);
            }
            Expr::UnaryOp(u) => self.mark_number(&u.operand),
            _ => {}
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
            // An item store of a number: `s[i] = (x + y) & 0xff`.
            Stmt::Assign(assign) => {
                if is_number_root(&assign.value) {
                    for target in &assign.targets {
                        if let Expr::Subscript(sub) = target
                            && !sub.slice.is_slice_expr()
                        {
                            self.mark(&sub.value, |u| u.byte_item = true);
                        }
                    }
                }
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
                        "abs" => {
                            if let [arg] = &call.arguments.args[..] {
                                self.mark_number(arg);
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
                if is_number_root(expr) {
                    self.mark_number(&bin.left);
                    self.mark_number(&bin.right);
                }
            }
            Expr::UnaryOp(u)
                if matches!(
                    u.op,
                    ruff_python_ast::UnaryOp::USub | ruff_python_ast::UnaryOp::Invert
                ) =>
            {
                self.mark_number(&u.operand);
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
                        if int_literal(right) {
                            self.mark_number(left);
                        }
                        if int_literal(left) {
                            self.mark_number(right);
                        }
                    }
                    left = right;
                }
            }
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }
}

/// Whether `expr` is arithmetic only numbers have: an operator `str`
/// lacks, or `+` with an integer literal.
fn is_number_root(expr: &Expr) -> bool {
    let Expr::BinOp(bin) = expr else {
        return false;
    };
    matches!(
        bin.op,
        Operator::Sub
            | Operator::FloorDiv
            | Operator::BitAnd
            | Operator::BitOr
            | Operator::BitXor
            | Operator::LShift
            | Operator::RShift
    ) || (bin.op == Operator::Add && (int_literal(&bin.left) || int_literal(&bin.right)))
}

/// Whether `expr` is an integer literal.
fn int_literal(expr: &Expr) -> bool {
    matches!(expr, Expr::NumberLiteral(n) if matches!(n.value, Number::Int(_)))
}
