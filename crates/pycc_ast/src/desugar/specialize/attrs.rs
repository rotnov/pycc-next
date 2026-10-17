//! Attribute snapshots: the instance attributes a method's specialization
//! takes as `int` parameters (`super`'s module doc).
//!
//! An untyped class's attributes are CPython objects (`pycc_hir`'s
//! `class::method_attrs`), so arithmetic on `self.bitdepth` runs through
//! the C API even in a native specialization. When the method only reads
//! such an attribute, uses it as an integer, and reads it only while
//! nothing but native operations has run since the call began, reading it
//! once at the call is indistinguishable from reading it at each use: no
//! code of the program can have changed it in between. The specialization
//! then takes it as a parameter of its own (`_pycc_attr_<name>: int`), the
//! guard checks the attribute's type (`type(self.<name>) is int`), and the
//! call passes `self.<name>`. Only the attribute the body reads first is
//! taken, so the guard's read raises the `AttributeError` of an unset one
//! exactly where the body's own would.
//!
//! "Nothing but native operations" is judged on the body's own shapes,
//! conservatively: integer arithmetic and comparisons between values known
//! to be integers, `len`, `range`, `list`, `reversed`, `abs`, `min`, `max`
//! and `bytearray` of known values, items and slices of byte buffers and
//! lists of integers, a byte buffer's `append`/`extend` of integers, list
//! comprehensions of those, `is` tests, and reads of attributes that are
//! not methods of the class. Anything else -- a call of anything else, an
//! operation on a value of unknown type (which may run a dunder method), a
//! `try` -- ends the region; a read of a candidate after it (or in a loop
//! that holds such an operation anywhere) leaves that attribute an
//! attribute read. A branch ending in `return` or `raise` ends its own
//! region only.

use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{CmpOp, Expr, ExprContext, Number, Operator, Stmt, StmtFunctionDef};
use std::collections::{BTreeSet, HashMap, HashSet};

/// The builtins the region analysis takes as native calls; one the module
/// rebinds is not.
pub(super) const BUILTINS_READ: &[&str] = &[
    "len", "range", "list", "reversed", "abs", "min", "max", "int", "bytearray",
];

/// The prefix of a snapshot parameter's name.
pub(super) const ATTR_PREFIX: &str = "_pycc_attr_";

/// What a class body declares that an attribute read could reach instead of
/// an instance slot.
pub(super) struct ClassFacts {
    /// Names the class body binds: methods, properties, class attributes.
    members: HashSet<String>,
    /// Whether instance attribute reads are plain slot reads: no base
    /// class and no `__getattr__`/`__getattribute__`.
    plain: bool,
}

impl ClassFacts {
    pub(super) fn of(class: &ruff_python_ast::StmtClassDef) -> Self {
        let mut members = HashSet::new();
        for stmt in class.body.iter() {
            match stmt {
                Stmt::FunctionDef(def) => {
                    members.insert(def.name.to_string());
                }
                Stmt::Assign(assign) => {
                    for target in &assign.targets {
                        if let Expr::Name(n) = target {
                            members.insert(n.id.to_string());
                        }
                    }
                }
                Stmt::AnnAssign(assign) => {
                    if let Expr::Name(n) = assign.target.as_ref() {
                        members.insert(n.id.to_string());
                    }
                }
                _ => {}
            }
        }
        let plain = class.arguments.as_ref().is_none_or(|args| args.is_empty())
            && !members.contains("__getattr__")
            && !members.contains("__getattribute__");
        ClassFacts { members, plain }
    }
}

/// The attributes of `receiver` the method `def` reads that its
/// specialization may take as `int` snapshots (module doc), given the
/// specialization's parameter guesses `params` (name -> type).
pub(super) fn snapshot_attrs(
    def: &StmtFunctionDef,
    receiver: &str,
    class: &ClassFacts,
    params: &HashMap<String, &'static str>,
    module_binds: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    if !class.plain {
        return Vec::new();
    }
    let mut reads = Reads {
        receiver,
        loads: BTreeSet::new(),
        stores: HashSet::new(),
        escapes: false,
    };
    for stmt in def.body.iter() {
        reads.stmt(stmt);
    }
    if reads.escapes {
        return Vec::new();
    }
    // Only the attribute the body reads first: the guard reads it before
    // the call, where an unset one raises the `AttributeError` the body's
    // own first read would.
    let Some(first) = leading_read(&def.body, receiver) else {
        return Vec::new();
    };
    let candidates: Vec<String> = reads
        .loads
        .into_iter()
        .filter(|attr| {
            *attr == first
                && !reads.stores.contains(attr)
                && !class.members.contains(attr)
                && !attr.starts_with('_')
        })
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    // The integer evidence: the body with each candidate read as a name.
    let mut scratch: Vec<Stmt> = def.body.to_vec();
    replace_reads(&mut scratch, receiver, &candidates);
    let names: Vec<String> = candidates.iter().map(|a| format!("{ATTR_PREFIX}{a}")).collect();
    let evidence = super::Evidence::of(&scratch, &names);
    let ints: Vec<String> = candidates
        .into_iter()
        .filter(|attr| evidence.guess(&format!("{ATTR_PREFIX}{attr}"), false) == Some("int"))
        .collect();
    if ints.is_empty() {
        return Vec::new();
    }
    let mut region = Region {
        receiver,
        members: &class.members,
        snapshots: ints.iter().cloned().collect(),
        kinds: HashMap::new(),
        late: HashSet::new(),
        module_binds,
    };
    for (name, ty) in params {
        let kind = match *ty {
            "int" => Kind::Int,
            "bytearray" => Kind::Bytes,
            "str" => Kind::Str,
            _ => continue,
        };
        region.kinds.insert(name.clone(), kind);
    }
    let mut tainted = false;
    region.block(&scratch, &mut tainted);
    ints.into_iter()
        .filter(|attr| !region.late.contains(attr))
        .collect()
}

/// The attribute of `receiver` whose read is the first thing `body` does
/// (after its docstring): the leftmost operand of its first statement's
/// first expression, as in `if self.bitdepth == 8:`.
fn leading_read(body: &[Stmt], receiver: &str) -> Option<String> {
    let first = body
        .iter()
        .find(|stmt| !matches!(stmt, Stmt::Expr(e) if e.value.is_string_literal_expr()))?;
    let mut expr = match first {
        Stmt::If(s) => s.test.as_ref(),
        Stmt::Assign(a) => a.value.as_ref(),
        Stmt::Return(r) => r.value.as_deref()?,
        Stmt::Expr(e) => e.value.as_ref(),
        Stmt::Assert(a) => a.test.as_ref(),
        _ => return None,
    };
    loop {
        expr = match expr {
            Expr::Compare(c) => c.left.as_ref(),
            Expr::BinOp(b) => b.left.as_ref(),
            Expr::BoolOp(b) => b.values.first()?,
            Expr::Attribute(attr)
                if attr.ctx == ExprContext::Load
                    && matches!(attr.value.as_ref(), Expr::Name(n) if n.id.as_str() == receiver) =>
            {
                return Some(attr.attr.to_string());
            }
            _ => return None,
        };
    }
}

/// Rewrites `clone` (the specialization) to take the snapshots `attrs` of
/// `receiver`: each read becomes the parameter, appended last as `int`.
pub(super) fn take_snapshots(clone: &mut StmtFunctionDef, receiver: &str, attrs: &[String]) {
    replace_reads(&mut clone.body, receiver, attrs);
    for attr in attrs {
        let range = clone.name.range;
        let mut param = clone.parameters.args[0].clone();
        param.default = None;
        param.parameter.name = ruff_python_ast::Identifier::new(format!("{ATTR_PREFIX}{attr}"), range);
        param.parameter.annotation = Some(Box::new(super::name("int", ExprContext::Load, range)));
        clone.parameters.args.push(param);
    }
}

/// Every load of `receiver.<attr>` (an attribute of `attrs`) in `body`
/// replaced by the snapshot name (not inside nested scopes).
fn replace_reads(body: &mut [Stmt], receiver: &str, attrs: &[String]) {
    struct Replace<'a> {
        receiver: &'a str,
        attrs: &'a [String],
    }
    impl Transformer for Replace<'_> {
        fn visit_stmt(&self, stmt: &mut Stmt) {
            if !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
                transformer::walk_stmt(self, stmt);
            }
        }
        fn visit_expr(&self, expr: &mut Expr) {
            if let Expr::Attribute(attr) = expr
                && attr.ctx == ExprContext::Load
                && matches!(attr.value.as_ref(), Expr::Name(n) if n.id.as_str() == self.receiver)
                && self.attrs.iter().any(|a| a == attr.attr.as_str())
            {
                let range = attr.range;
                *expr = super::name(&format!("{ATTR_PREFIX}{}", attr.attr), ExprContext::Load, range);
                return;
            }
            if !matches!(expr, Expr::Lambda(_)) {
                transformer::walk_expr(self, expr);
            }
        }
    }
    let replace = Replace { receiver, attrs };
    for stmt in body {
        replace.visit_stmt(stmt);
    }
}

/// The receiver's attribute loads and stores, and whether the receiver is
/// used any other way that the snapshots cannot see through: rebound, or
/// read inside a nested function or lambda.
struct Reads<'a> {
    receiver: &'a str,
    loads: BTreeSet<String>,
    stores: HashSet<String>,
    escapes: bool,
}

impl Reads<'_> {
    fn stmt(&mut self, stmt: &Stmt) {
        use ruff_python_ast::visitor::{self, Visitor};
        struct Walk<'r, 'a>(&'r mut Reads<'a>);
        impl<'b> Visitor<'b> for Walk<'_, '_> {
            fn visit_stmt(&mut self, stmt: &'b Stmt) {
                match stmt {
                    Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                        self.0.escapes = true;
                        return;
                    }
                    Stmt::Delete(delete) => {
                        for target in &delete.targets {
                            self.store(target);
                        }
                    }
                    Stmt::Assign(assign) => assign.targets.iter().for_each(|t| self.store(t)),
                    Stmt::AugAssign(aug) => self.store(&aug.target),
                    Stmt::AnnAssign(ann) => self.store(&ann.target),
                    Stmt::For(f) => self.store(&f.target),
                    Stmt::With(w) => {
                        for item in &w.items {
                            if let Some(var) = &item.optional_vars {
                                self.store(var);
                            }
                        }
                    }
                    _ => {}
                }
                visitor::walk_stmt(self, stmt);
            }
            fn visit_expr(&mut self, expr: &'b Expr) {
                match expr {
                    Expr::Lambda(_) => {
                        self.0.escapes = true;
                        return;
                    }
                    Expr::Named(named) => self.store(&named.target),
                    Expr::Attribute(attr)
                        if attr.ctx == ExprContext::Load
                            && matches!(attr.value.as_ref(), Expr::Name(n) if n.id.as_str() == self.0.receiver) =>
                    {
                        self.0.loads.insert(attr.attr.to_string());
                        return;
                    }
                    _ => {}
                }
                visitor::walk_expr(self, expr);
            }
        }
        impl Walk<'_, '_> {
            fn store(&mut self, target: &Expr) {
                match target {
                    Expr::Name(n) if n.id.as_str() == self.0.receiver => self.0.escapes = true,
                    Expr::Attribute(attr)
                        if matches!(attr.value.as_ref(), Expr::Name(n) if n.id.as_str() == self.0.receiver) =>
                    {
                        self.0.stores.insert(attr.attr.to_string());
                    }
                    Expr::Tuple(t) => t.elts.iter().for_each(|e| self.store(e)),
                    Expr::List(l) => l.elts.iter().for_each(|e| self.store(e)),
                    Expr::Starred(s) => self.store(&s.value),
                    _ => {}
                }
            }
        }
        Walk(self).visit_stmt(stmt);
    }
}

/// What the region analysis knows a value to be.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Int,
    Float,
    Bool,
    Str,
    None,
    /// A byte buffer: a guessed `bytearray` parameter or a `bytearray()`.
    Bytes,
    /// A list of integers the body built itself.
    Ints,
    /// `range(...)` or `reversed(...)` of a list of integers.
    IntIter,
    /// A value whose type the analysis does not know; reading it is native,
    /// operating on it may not be.
    Unknown,
}

/// The region analysis (module doc).
struct Region<'a> {
    receiver: &'a str,
    members: &'a HashSet<String>,
    /// The candidate attributes, read as `_pycc_attr_<name>`.
    snapshots: HashSet<String>,
    kinds: HashMap<String, Kind>,
    /// The candidates read after the region ended.
    late: HashSet<String>,
    module_binds: &'a dyn Fn(&str) -> bool,
}

impl Region<'_> {
    /// Analyzes `body`; `tainted` is whether the region has ended on the
    /// path reaching it, and is updated to whether it has after it.
    /// Returns whether `body` always ends in `return` or `raise`.
    fn block(&mut self, body: &[Stmt], tainted: &mut bool) -> bool {
        for stmt in body {
            if self.stmt(stmt, tainted) {
                return true;
            }
        }
        false
    }

    /// Records the candidate reads in `expr` as late when `tainted`.
    fn reads(&mut self, expr: &Expr, tainted: bool) {
        use ruff_python_ast::visitor::{self, Visitor};
        struct Find<'s>(&'s HashSet<String>, Vec<String>);
        impl<'b> Visitor<'b> for Find<'_> {
            fn visit_expr(&mut self, expr: &'b Expr) {
                if let Expr::Name(n) = expr
                    && let Some(attr) = n.id.as_str().strip_prefix(ATTR_PREFIX)
                    && self.0.contains(attr)
                {
                    self.1.push(attr.to_string());
                }
                visitor::walk_expr(self, expr);
            }
        }
        if !tainted {
            return;
        }
        let mut find = Find(&self.snapshots, Vec::new());
        find.visit_expr(expr);
        self.late.extend(find.1);
    }

    /// Every candidate read anywhere in `stmt` is late when `tainted`.
    fn stmt_reads(&mut self, stmt: &Stmt, tainted: bool) {
        use ruff_python_ast::visitor::{self, Visitor};
        struct All<'r, 'a>(&'r mut Region<'a>, bool);
        impl<'b> Visitor<'b> for All<'_, '_> {
            fn visit_expr(&mut self, expr: &'b Expr) {
                self.0.reads(expr, self.1);
            }
        }
        let mut all = All(self, tainted);
        visitor::walk_stmt(&mut all, stmt);
    }

    /// One statement; returns whether it always ends the body (`return`,
    /// `raise`).
    fn stmt(&mut self, stmt: &Stmt, tainted: &mut bool) -> bool {
        match stmt {
            Stmt::Expr(e) if e.value.is_string_literal_expr() => false,
            Stmt::Pass(_) | Stmt::Break(_) | Stmt::Continue(_) => false,
            Stmt::Assign(assign) => {
                let kind = self.value(&assign.value, *tainted);
                match (assign.targets.as_slice(), kind) {
                    ([Expr::Name(target)], Some(kind)) => {
                        self.kinds.insert(target.id.to_string(), kind);
                    }
                    _ => {
                        for target in &assign.targets {
                            self.forget(target);
                        }
                        *tainted = true;
                    }
                }
                false
            }
            Stmt::AugAssign(aug) => {
                let Expr::Name(target) = aug.target.as_ref() else {
                    self.stmt_reads(stmt, true);
                    *tainted = true;
                    return false;
                };
                let left = self.kinds.get(target.id.as_str()).copied();
                let right = self.value(&aug.value, *tainted);
                match (left, right) {
                    (Some(left), Some(right)) => match arith(aug.op, left, right) {
                        Some(kind) => {
                            self.kinds.insert(target.id.to_string(), kind);
                        }
                        None => {
                            self.kinds.remove(target.id.as_str());
                            *tainted = true;
                        }
                    },
                    _ => {
                        self.kinds.remove(target.id.as_str());
                        *tainted = true;
                    }
                }
                false
            }
            Stmt::Expr(e) => {
                if self.value(&e.value, *tainted).is_none() {
                    *tainted = true;
                }
                false
            }
            Stmt::Assert(a) => {
                if self.value(&a.test, *tainted).is_none() {
                    *tainted = true;
                }
                if let Some(msg) = &a.msg {
                    self.reads(msg, true);
                }
                false
            }
            Stmt::Return(r) => {
                if let Some(value) = &r.value
                    && self.value(value, *tainted).is_none()
                {
                    // Reads in an operand evaluated after the unknown one
                    // would be late; any read here is counted late.
                    self.reads(value, true);
                }
                true
            }
            Stmt::Raise(r) => {
                if let Some(exc) = &r.exc {
                    self.reads(exc, true);
                }
                true
            }
            Stmt::If(s) => {
                if self.value(&s.test, *tainted).is_none() {
                    *tainted = true;
                }
                let before = *tainted;
                let saved = self.kinds.clone();
                let mut after = false;
                let mut all_end = true;
                let mut branch = before;
                let ends = self.block(&s.body, &mut branch);
                if !ends {
                    after |= branch;
                }
                all_end &= ends;
                let mut has_else = false;
                for clause in &s.elif_else_clauses {
                    if let Some(test) = &clause.test {
                        let mut t = before;
                        if self.value(test, t).is_none() {
                            t = true;
                        }
                        let mut branch = t;
                        let ends = self.block(&clause.body, &mut branch);
                        if !ends {
                            after |= branch;
                        }
                        all_end &= ends;
                        // A later clause runs after this test.
                        if t {
                            after = true;
                        }
                    } else {
                        has_else = true;
                        let mut branch = before;
                        let ends = self.block(&clause.body, &mut branch);
                        if !ends {
                            after |= branch;
                        }
                        all_end &= ends;
                    }
                }
                if !has_else {
                    after |= before;
                    all_end = false;
                }
                // A name bound differently by the branches is unknown.
                let bound: Vec<String> = self.kinds.keys().cloned().collect();
                for name in bound {
                    if saved.get(&name) != self.kinds.get(&name) {
                        self.kinds.remove(&name);
                    }
                }
                *tainted = after;
                all_end
            }
            Stmt::For(f) => {
                let item = match self.value(&f.iter, *tainted) {
                    Some(Kind::Bytes | Kind::Ints | Kind::IntIter) => Some(Kind::Int),
                    Some(Kind::Str) => Some(Kind::Str),
                    _ => None,
                };
                let Expr::Name(target) = f.target.as_ref() else {
                    self.stmt_reads(stmt, true);
                    *tainted = true;
                    return false;
                };
                if item.is_none() {
                    *tainted = true;
                }
                self.loop_body(&f.body, item.map(|k| (target.id.to_string(), k)), tainted);
                self.block(&f.orelse, tainted);
                false
            }
            Stmt::While(w) => {
                if self.value(&w.test, *tainted).is_none() {
                    *tainted = true;
                }
                self.loop_body(&w.body, None, tainted);
                // The test runs again after the body.
                if *tainted {
                    self.reads(&w.test, true);
                }
                self.block(&w.orelse, tainted);
                false
            }
            other => {
                self.stmt_reads(other, true);
                *tainted = true;
                false
            }
        }
    }

    /// A loop body, run any number of times: what ends the region in one
    /// iteration ends it for the reads of the next. The names the body
    /// binds have no known kind in it (a later iteration may see another
    /// binding) nor after it.
    fn loop_body(&mut self, body: &[Stmt], var: Option<(String, Kind)>, tainted: &mut bool) {
        let enter = |region: &mut Self| {
            for stmt in body {
                forget_bound(stmt, &mut region.kinds);
            }
            if let Some((name, kind)) = &var {
                region.kinds.insert(name.clone(), *kind);
            }
        };
        enter(self);
        let mut after = *tainted;
        self.block(body, &mut after);
        if after && !*tainted {
            enter(self);
            let mut again = true;
            self.block(body, &mut again);
        }
        for stmt in body {
            forget_bound(stmt, &mut self.kinds);
        }
        *tainted = after;
    }

    fn forget(&mut self, target: &Expr) {
        if let Expr::Name(n) = target {
            self.kinds.remove(n.id.as_str());
        }
    }

    /// The kind of `expr` when evaluating it runs only native operations,
    /// recording late candidate reads; `None` otherwise.
    fn value(&mut self, expr: &Expr, tainted: bool) -> Option<Kind> {
        let kind = self.kind(expr);
        // Reads in an expression that may run program code count as late:
        // the analysis does not order the operands' evaluation.
        self.reads(expr, tainted || kind.is_none());
        kind
    }

    fn kind(&self, expr: &Expr) -> Option<Kind> {
        match expr {
            Expr::NumberLiteral(n) => Some(match n.value {
                Number::Int(_) => Kind::Int,
                Number::Float(_) => Kind::Float,
                Number::Complex { .. } => return None,
            }),
            Expr::BooleanLiteral(_) => Some(Kind::Bool),
            Expr::StringLiteral(_) => Some(Kind::Str),
            Expr::NoneLiteral(_) => Some(Kind::None),
            Expr::Name(n) => {
                if let Some(attr) = n.id.as_str().strip_prefix(ATTR_PREFIX)
                    && self.snapshots.contains(attr)
                {
                    return Some(Kind::Int);
                }
                Some(self.kinds.get(n.id.as_str()).copied().unwrap_or(Kind::Unknown))
            }
            // A plain instance slot: its read runs no program code.
            Expr::Attribute(attr)
                if attr.ctx == ExprContext::Load
                    && matches!(attr.value.as_ref(), Expr::Name(n) if n.id.as_str() == self.receiver)
                    && !self.members.contains(attr.attr.as_str()) =>
            {
                Some(Kind::Unknown)
            }
            Expr::UnaryOp(u) => {
                let operand = self.kind(&u.operand)?;
                match (u.op, operand) {
                    (ruff_python_ast::UnaryOp::Not, k) if k != Kind::Unknown => Some(Kind::Bool),
                    (_, Kind::Int) => Some(Kind::Int),
                    _ => None,
                }
            }
            Expr::BinOp(bin) => arith(bin.op, self.kind(&bin.left)?, self.kind(&bin.right)?),
            Expr::BoolOp(b) => {
                for value in &b.values {
                    let kind = self.kind(value)?;
                    if !matches!(kind, Kind::Int | Kind::Bool | Kind::None) {
                        return None;
                    }
                }
                Some(Kind::Unknown)
            }
            Expr::Compare(c) => {
                let mut left = self.kind(&c.left)?;
                for (op, right) in c.ops.iter().zip(c.comparators.iter()) {
                    let right_kind = self.kind(right)?;
                    let ok = match op {
                        CmpOp::Is | CmpOp::IsNot => true,
                        _ => {
                            let numeric = |k: Kind| matches!(k, Kind::Int | Kind::Bool | Kind::Float);
                            (numeric(left) && numeric(right_kind))
                                || (left == Kind::Str && right_kind == Kind::Str)
                        }
                    };
                    if !ok {
                        return None;
                    }
                    left = right_kind;
                }
                Some(Kind::Bool)
            }
            Expr::Subscript(sub) => {
                let base = self.kind(&sub.value)?;
                if let Expr::Slice(slice) = sub.slice.as_ref() {
                    for bound in [&slice.lower, &slice.upper, &slice.step].into_iter().flatten() {
                        if !matches!(self.kind(bound)?, Kind::Int | Kind::None) {
                            return None;
                        }
                    }
                    return match base {
                        Kind::Bytes | Kind::Ints | Kind::Str => Some(base),
                        _ => None,
                    };
                }
                if self.kind(&sub.slice)? != Kind::Int {
                    return None;
                }
                match base {
                    Kind::Bytes | Kind::Ints => Some(Kind::Int),
                    Kind::Str => Some(Kind::Str),
                    _ => None,
                }
            }
            Expr::ListComp(comp) => {
                let [generator] = comp.generators.as_slice() else {
                    return None;
                };
                if !generator.ifs.is_empty() || generator.is_async {
                    return None;
                }
                let item = match self.kind(&generator.iter)? {
                    Kind::Bytes | Kind::Ints | Kind::IntIter => Kind::Int,
                    _ => return None,
                };
                let Expr::Name(var) = &generator.target else {
                    return None;
                };
                let mut inner = Region {
                    receiver: self.receiver,
                    members: self.members,
                    snapshots: self.snapshots.clone(),
                    kinds: self.kinds.clone(),
                    late: HashSet::new(),
                    module_binds: self.module_binds,
                };
                inner.kinds.insert(var.id.to_string(), item);
                match inner.kind(&comp.elt)? {
                    Kind::Int => Some(Kind::Ints),
                    _ => None,
                }
            }
            Expr::List(list) => {
                for element in &list.elts {
                    if self.kind(element)? != Kind::Int {
                        return None;
                    }
                }
                Some(Kind::Ints)
            }
            Expr::Call(call) if call.arguments.keywords.is_empty() => {
                let args: Vec<Kind> = call
                    .arguments
                    .args
                    .iter()
                    .map(|arg| self.kind(arg))
                    .collect::<Option<_>>()?;
                match call.func.as_ref() {
                    Expr::Attribute(attr) if attr.attr.as_str() == "append" || attr.attr.as_str() == "extend" => {
                        let base = self.kind(&attr.value)?;
                        match (base, attr.attr.as_str(), args.as_slice()) {
                            (Kind::Bytes, "append", [Kind::Int]) => Some(Kind::None),
                            (Kind::Bytes, "extend", [Kind::Ints | Kind::Bytes]) => Some(Kind::None),
                            _ => None,
                        }
                    }
                    func => builtin(self.builtin_name(func)?, &args),
                }
            }
            _ => None,
        }
    }

    /// The builtin `func` names: a bare builtin name the module does not
    /// bind, or one read through the fallback planner's `builtins` module.
    fn builtin_name<'e>(&self, func: &'e Expr) -> Option<&'e str> {
        match func {
            Expr::Name(n) if !self.kinds.contains_key(n.id.as_str()) && !(self.module_binds)(n.id.as_str()) => {
                Some(n.id.as_str())
            }
            Expr::Attribute(attr)
                if matches!(attr.value.as_ref(), Expr::Name(n) if n.id.as_str() == super::FALLBACK_BUILTINS) =>
            {
                Some(attr.attr.as_str())
            }
            _ => None,
        }
    }
}

/// The kind of `left op right` when both are numbers the operator only
/// computes on; `None` when it may run program code.
fn arith(op: Operator, left: Kind, right: Kind) -> Option<Kind> {
    let int = |k: Kind| matches!(k, Kind::Int | Kind::Bool);
    match op {
        Operator::Add | Operator::Sub | Operator::Mult | Operator::FloorDiv | Operator::Mod
        | Operator::LShift | Operator::RShift | Operator::BitAnd | Operator::BitOr
        | Operator::BitXor | Operator::Pow
            if int(left) && int(right) =>
        {
            Some(Kind::Int)
        }
        Operator::Div if int(left) && int(right) => Some(Kind::Float),
        _ => None,
    }
}

/// The kind of a call of the builtin `name` with arguments of `args` kinds,
/// when the call runs only native code.
fn builtin(name: &str, args: &[Kind]) -> Option<Kind> {
    let int = |k: &Kind| matches!(k, Kind::Int | Kind::Bool);
    Some(match (name, args) {
        ("len", [Kind::Bytes | Kind::Ints | Kind::Str]) => Kind::Int,
        ("range", args) if (1..=3).contains(&args.len()) && args.iter().all(int) => Kind::IntIter,
        ("list", [Kind::IntIter | Kind::Ints | Kind::Bytes]) => Kind::Ints,
        ("reversed", [Kind::Ints]) => Kind::IntIter,
        ("abs", [k]) if int(k) => Kind::Int,
        ("min" | "max", args) if args.len() >= 2 && args.iter().all(int) => Kind::Int,
        ("int", [k]) if int(k) => Kind::Int,
        ("bytearray", []) => Kind::Bytes,
        ("bytearray", [k]) if int(k) || *k == Kind::Bytes => Kind::Bytes,
        _ => return None,
    })
}

/// Forgets the kinds of the names `stmt` binds.
fn forget_bound(stmt: &Stmt, kinds: &mut HashMap<String, Kind>) {
    for name in super::super::stored_names(std::slice::from_ref(stmt)) {
        kinds.remove(&name);
    }
}
