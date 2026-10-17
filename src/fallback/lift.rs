//! Method-level fallback (v0.7 accept 3) and classes CPython creates
//! (accept 2): the `def`s inside a top-level statement that runs in CPython
//! are *lifted* out of it and compiled on their own.
//!
//! A class statement falls back as a whole when anything in it is beyond
//! the compiler: a base that is a CPython class (`class M(lib.Base)`), a
//! metaclass, a class decorator or attribute pycc cannot lower, a decorated
//! method -- or one method whose body uses something unsupported. So does a
//! module-level function (for its decorators, or a signature pycc cannot
//! compile), and a module-level block (`if ...:`, `try:`) defining
//! functions. Running all of that in CPython would
//! interpret every method of the class; instead the statement is *split*:
//!
//! * **CPython runs the statement**, exactly as written but for the bodies
//!   of its lifted `def`s: the class object is created by CPython, so its
//!   bases, metaclass (`__prepare__`, `__new__`, `__init_subclass__`),
//!   `__set_name__` hooks, class attributes, decorators (`@property`,
//!   `@classmethod`, `@functools.wraps`-style and library decorators),
//!   parameter defaults and annotations all behave as CPython's own. Each
//!   lifted method keeps its name, signature, docstring and decorators;
//!   its body is one line, a *forwarder*: `return __pycc_liftN_m__(self,
//!   a, b, args, kw)`, passing every parameter positionally (`*args` and
//!   `**kwargs` as the tuple and the dict).
//! * **pycc compiles each body** as a module-level function,
//!   `__pycc_liftN_m__`, every parameter and its return an `object`: `self`
//!   is the CPython instance, and attribute access on it goes through the
//!   object protocol. The module publishes the function (an `--ext` module,
//!   or an embedded executable's `__main__`), so the forwarder finds it in
//!   the module's namespace. A body that reads `__class__` or calls
//!   `super()` with no arguments receives the class too: the forwarder
//!   passes its own `__class__` cell, and `super()` becomes
//!   `super(__pycc_class__, self)`. A class-private attribute (`self.__x`)
//!   is written mangled (`self._C__x`), as CPython compiles it in the class
//!   body.
//!
//! A lifted body the compiler refuses falls back on its own: its `def`
//! stays as written in the CPython statement (`--report-fallbacks` names
//! it, `method C.m`), and every other method stays compiled. So the
//! fallback granularity is the smallest enclosing function or method.
//!
//! The compiled function is defined (and published) before CPython runs
//! the statement, so a metaclass or decorator that calls a method while
//! the class is created finds it: in the rewrite each lifted body sits at
//! its own lines, and the statement's `exec` follows the last of them.
//!
//! Not lifted (the `def` runs in CPython with its statement): an `async
//! def`, a generic `def f[T]()`, a body that is only a docstring, a body
//! using `super()` or `__class__` inside a nested function, lambda or
//! comprehension (whose first argument is not the method's), and a body
//! reading a module global CPython would mangle (`__x` in a class).

use super::*;
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, StmtFunctionDef};

/// The prefix of a lifted function's name. With the `__` suffix every
/// lifted name ends in, CPython never mangles it in a class body.
pub(crate) const LIFT_PREFIX: &str = "__pycc_lift";

/// The parameter a lifted body that reads `__class__` or calls `super()`
/// receives the class in.
const CLASS_PARAM: &str = "__pycc_class__";

/// One `def` lifted out of the top-level statement `owner`.
#[derive(Debug, Clone)]
pub(super) struct Lift {
    /// The top-level statement holding it.
    pub(super) owner: usize,
    /// `C.m`, `m` (a module-level `def` in a block), for the report.
    pub(super) qualname: String,
    /// The compiled function's name (`__pycc_liftN_m__`).
    pub(super) name: String,
    /// The `def`'s range in the source, decorators included.
    pub(super) range: Range<u32>,
    /// Where its `def` keyword starts: the compiled function's text
    /// starts there.
    pub(super) keyword: u32,
    /// What the forwarder replaces in the CPython statement: the body after
    /// its docstring.
    pub(super) body: Range<u32>,
    /// The parameter names, in order: what the forwarder passes.
    pub(super) params: Vec<String>,
    /// Whether the forwarder passes the class (`__class__`).
    pub(super) uses_class: bool,
    /// The compiled function's own edits of the `def`'s source: its name,
    /// its signature, `super()` and `__class__`, mangled names.
    pub(super) edits: Vec<(Range<u32>, String)>,
    /// Whether the body needs no edit of its own (no `super()`, `__class__`
    /// or mangled name): it runs in CPython as written (`attach`).
    pub(super) plain_body: bool,
    /// The `:` ending the `def`'s header.
    pub(super) colon: u32,
    /// How the method attaches to its compiled class when it falls back
    /// alone (`attach`), if it can.
    pub(super) attach: Option<super::attach::Attach>,
}

impl Lift {
    /// The forwarder replacing the body in CPython code.
    fn forwarder(&self) -> String {
        let mut args: Vec<&str> = Vec::new();
        if self.uses_class {
            args.push("__class__");
        }
        args.extend(self.params.iter().map(String::as_str));
        format!("return {}({})", self.name, args.join(", "))
    }
}

/// The `def`s of the top-level statement `stmt` (statement `owner`) that
/// lift when it runs in CPython, each with the statement the compiler
/// analyzes for it (the `def` without decorators, defaults or annotations).
/// `next` numbers the lifted names across the module.
pub(super) fn find(
    owner: usize,
    stmt: &Stmt,
    source: &str,
    next: &mut usize,
) -> Vec<(Lift, Stmt)> {
    let mut finder = Finder {
        owner,
        source,
        next,
        found: Vec::new(),
    };
    // A module-level function lifts too: what CPython cannot compile may be
    // its decorators or its signature (a default, `**kwargs`), which the
    // forwarder keeps, while its body compiles.
    finder.body(std::slice::from_ref(stmt), None, "");
    finder.found
}

struct Finder<'s, 'n> {
    owner: usize,
    source: &'s str,
    next: &'n mut usize,
    found: Vec<(Lift, Stmt)>,
}

impl Finder<'_, '_> {
    /// Looks for `def`s in `body`, a class body (`class` names it) or a
    /// block, without entering a function.
    fn body(&mut self, body: &[Stmt], class: Option<&str>, qual: &str) {
        for stmt in body {
            match stmt {
                Stmt::FunctionDef(def) => self.def(def, class, qual),
                Stmt::ClassDef(inner) => {
                    let name = inner.name.as_str();
                    self.body(&inner.body, Some(name), &format!("{qual}{name}."));
                }
                Stmt::If(s) => {
                    self.body(&s.body, class, qual);
                    for clause in &s.elif_else_clauses {
                        self.body(&clause.body, class, qual);
                    }
                }
                Stmt::For(s) => {
                    self.body(&s.body, class, qual);
                    self.body(&s.orelse, class, qual);
                }
                Stmt::While(s) => {
                    self.body(&s.body, class, qual);
                    self.body(&s.orelse, class, qual);
                }
                Stmt::With(s) => self.body(&s.body, class, qual),
                Stmt::Try(s) => {
                    self.body(&s.body, class, qual);
                    for handler in &s.handlers {
                        let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
                        self.body(&h.body, class, qual);
                    }
                    self.body(&s.orelse, class, qual);
                    self.body(&s.finalbody, class, qual);
                }
                Stmt::Match(s) => {
                    for case in &s.cases {
                        self.body(&case.body, class, qual);
                    }
                }
                _ => {}
            }
        }
    }

    /// Lifts `def` (in class `class`, if any) when it can be.
    fn def(&mut self, def: &StmtFunctionDef, class: Option<&str>, qual: &str) {
        if def.is_async || def.type_params.is_some() {
            return;
        }
        let skip = usize::from(matches!(def.body.first(), Some(Stmt::Expr(e))
            if e.value.is_string_literal_expr()));
        let Some(first) = def.body.get(skip) else {
            return;
        };
        let range: Range<u32> = def.range.into();
        let name_start = def.name.range.start().to_u32();
        let Some(keyword) = self.source[range.start as usize..name_start as usize]
            .rfind("def")
            .map(|at| range.start + at as u32)
        else {
            return;
        };
        let params = parameter_order(&def.parameters);
        if params.iter().any(|p| p == CLASS_PARAM) {
            return;
        }
        // The body's own names: a `__x` among them is consistent unmangled.
        let mut locals = names::bound_names(&def.body);
        locals.extend(params.iter().cloned());
        let prefix = class
            .map(|class| class.trim_start_matches('_'))
            .filter(|stripped| !stripped.is_empty())
            .map(|stripped| format!("_{stripped}"));
        let mut scan = BodyScan {
            receiver: params.first().map(String::as_str).unwrap_or(""),
            prefix: prefix.as_deref(),
            locals: &locals,
            depth: 0,
            refused: false,
            uses_class: false,
            edits: Vec::new(),
        };
        for stmt in &def.body {
            scan.visit_stmt(stmt);
        }
        if scan.refused || (scan.uses_class && params.is_empty()) {
            return;
        }
        let uses_class = scan.uses_class;
        let plain_body = scan.edits.is_empty();
        let index = *self.next;
        *self.next += 1;
        let name = format!("{LIFT_PREFIX}{index}_{}__", def.name.as_str());
        let mut edits = scan.edits;
        edits.push((
            def.name.range.start().to_u32()..def.name.range.end().to_u32(),
            name.clone(),
        ));
        // The signature: every parameter positional and an object, the
        // class first when the body needs it. Its line breaks stay inside
        // the parentheses, so the body keeps its line numbers.
        let parameters: Range<u32> = def.parameters.range.into();
        let Some(signature_end) = header_colon(self.source, parameters.end as usize) else {
            return;
        };
        let signature_end = signature_end as u32;
        let lines = self.source[parameters.start as usize..signature_end as usize]
            .matches('\n')
            .count();
        let mut list: Vec<String> = Vec::new();
        if uses_class {
            list.push(format!("{CLASS_PARAM}: object"));
        }
        list.extend(params.iter().map(|p| format!("{p}: object")));
        edits.push((
            parameters.start..signature_end,
            format!("({}{}) -> object", list.join(", "), "\n".repeat(lines)),
        ));
        let body = pycc_ast::stmt_range(first).start..range.end;
        let qualname = format!("{qual}{}", def.name.as_str());
        let lift = Lift {
            owner: self.owner,
            qualname,
            name,
            range,
            keyword,
            body,
            params,
            uses_class,
            edits,
            plain_body,
            colon: signature_end,
            attach: None,
        };
        self.found.push((lift, Stmt::FunctionDef(analyzed(def))));
    }
}

/// The offset of the `:` ending a `def`'s header, scanning from `from`
/// (the end of its parameter list) past the return annotation: the first
/// `:` outside brackets, strings and comments.
fn header_colon(source: &str, from: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut at = from;
    while at < bytes.len() {
        match bytes[at] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b':' if depth == 0 => return Some(at),
            b'#' => {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
                continue;
            }
            quote @ (b'\'' | b'"') => {
                let triple = bytes.get(at..at + 3) == Some(&[quote, quote, quote][..]);
                let width = if triple { 3 } else { 1 };
                at += width;
                loop {
                    if at >= bytes.len() {
                        return None;
                    }
                    if bytes[at] == b'\\' {
                        at += 2;
                        continue;
                    }
                    if bytes[at] == quote
                        && (!triple || bytes.get(at..at + 3) == Some(&[quote, quote, quote][..]))
                    {
                        at += width;
                        break;
                    }
                    at += 1;
                }
                continue;
            }
            _ => {}
        }
        at += 1;
    }
    None
}

/// `def`'s parameter names in order: positional-only, positional,
/// `*args`, keyword-only, `**kwargs`.
fn parameter_order(parameters: &pycc_ast::Parameters) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let name = |p: &pycc_ast::ParameterWithDefault| p.parameter.name.as_str().to_string();
    out.extend(parameters.posonlyargs.iter().map(name));
    out.extend(parameters.args.iter().map(name));
    if let Some(vararg) = &parameters.vararg {
        out.push(vararg.name.as_str().to_string());
    }
    out.extend(parameters.kwonlyargs.iter().map(name));
    if let Some(kwarg) = &parameters.kwarg {
        out.push(kwarg.name.as_str().to_string());
    }
    out
}

/// The `def` as the compiled function's analysis sees it: no decorators,
/// defaults or annotations, which CPython evaluates in the statement.
fn analyzed(def: &StmtFunctionDef) -> StmtFunctionDef {
    let mut def = def.clone();
    def.decorator_list.clear();
    def.returns = None;
    let parameters = &mut *def.parameters;
    for p in parameters
        .posonlyargs
        .iter_mut()
        .chain(parameters.args.iter_mut())
        .chain(parameters.kwonlyargs.iter_mut())
    {
        p.default = None;
        p.parameter.annotation = None;
    }
    for p in parameters.vararg.iter_mut().chain(parameters.kwarg.iter_mut()) {
        p.annotation = None;
    }
    def
}

/// The scan of a lifted body: `super()` and `__class__`, and the names
/// CPython mangles.
struct BodyScan<'p> {
    /// The `def`'s first parameter, the receiver `super()` binds.
    receiver: &'p str,
    /// `_C`, the mangling prefix of the enclosing class.
    prefix: Option<&'p str>,
    /// The body's parameters and local bindings.
    locals: &'p BTreeSet<String>,
    /// Nested function, lambda, class and comprehension scopes entered.
    depth: usize,
    refused: bool,
    uses_class: bool,
    edits: Vec<(Range<u32>, String)>,
}

/// Whether CPython mangles `name` in a class body: `__x`, not `__x__`.
fn is_private(name: &str) -> bool {
    name.starts_with("__") && !name.ends_with("__")
}

impl BodyScan<'_> {
    fn mangle(&mut self, name: &str, range: Range<u32>) {
        if let Some(prefix) = self.prefix
            && is_private(name)
        {
            self.edits.push((range, format!("{prefix}{name}")));
        }
    }

    fn nested(&mut self, walk: impl FnOnce(&mut Self)) {
        self.depth += 1;
        walk(self);
        self.depth -= 1;
    }
}

impl<'a> Visitor<'a> for BodyScan<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                self.nested(|scan| visitor::walk_stmt(scan, stmt));
            }
            Stmt::Global(_) | Stmt::Nonlocal(_) => {
                // A `global __x` names the mangled global.
                if let Stmt::Global(global) = stmt
                    && self.prefix.is_some()
                    && global.names.iter().any(|n| is_private(n.as_str()))
                {
                    self.refused = true;
                }
                if matches!(stmt, Stmt::Nonlocal(_)) && self.depth == 0 {
                    self.refused = true;
                }
            }
            Stmt::Import(_) | Stmt::ImportFrom(_) if self.prefix.is_some() => {
                // `import __x` binds a mangled name: rare, not lifted.
                let mut names = names::bound_names(std::slice::from_ref(stmt)).into_iter();
                if names.any(|n| is_private(&n)) {
                    self.refused = true;
                }
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Call(call)
                if matches!(&*call.func, Expr::Name(n) if n.id.as_str() == "super")
                    && call.arguments.args.is_empty()
                    && call.arguments.keywords.is_empty() =>
            {
                if self.depth > 0 {
                    self.refused = true;
                }
                self.uses_class = true;
                self.edits.push((
                    pycc_ast::expr_range(expr),
                    format!("{BUILTINS}.super({CLASS_PARAM}, {})", self.receiver),
                ));
            }
            Expr::Name(name) if name.id.as_str() == "__class__" => {
                if self.depth > 0 {
                    self.refused = true;
                }
                self.uses_class = true;
                self.edits
                    .push((pycc_ast::expr_range(expr), CLASS_PARAM.to_string()));
            }
            Expr::Name(name) => {
                // A private name that is not the body's own is a mangled
                // global: not lifted.
                if self.prefix.is_some()
                    && is_private(name.id.as_str())
                    && (self.depth > 0 || !self.locals.contains(name.id.as_str()))
                {
                    self.refused = true;
                }
            }
            Expr::Attribute(attr) => {
                self.mangle(
                    attr.attr.as_str(),
                    attr.attr.range.start().to_u32()..attr.attr.range.end().to_u32(),
                );
                visitor::walk_expr(self, expr);
            }
            // A list, set or dict comprehension runs in its function's own
            // frame (PEP 709): `super()` there is the method's. A generator
            // expression or a lambda has a frame of its own.
            Expr::Lambda(_) | Expr::Generator(_) => {
                self.nested(|scan| visitor::walk_expr(scan, expr));
            }
            _ => visitor::walk_expr(self, expr),
        }
    }

    fn visit_keyword(&mut self, keyword: &'a pycc_ast::Keyword) {
        if let Some(arg) = &keyword.arg {
            self.mangle(
                arg.as_str(),
                arg.range.start().to_u32()..arg.range.end().to_u32(),
            );
        }
        visitor::walk_keyword(self, keyword);
    }
}

/// The analyzed form of top-level statement `stmt` with every lifted
/// `def`'s body removed: what CPython runs of it once split.
pub(super) fn without_bodies(stmt: &Stmt, lifted: &BTreeSet<u32>) -> Stmt {
    let mut stmt = stmt.clone();
    strip(std::slice::from_mut(&mut stmt), lifted);
    stmt
}

fn strip(body: &mut [Stmt], lifted: &BTreeSet<u32>) {
    for stmt in body {
        match stmt {
            Stmt::FunctionDef(def) => {
                if lifted.contains(&def.range.start().to_u32()) {
                    def.body.clear();
                }
            }
            Stmt::ClassDef(class) => strip(&mut class.body, lifted),
            Stmt::If(s) => {
                strip(&mut s.body, lifted);
                for clause in &mut s.elif_else_clauses {
                    strip(&mut clause.body, lifted);
                }
            }
            Stmt::For(s) => {
                strip(&mut s.body, lifted);
                strip(&mut s.orelse, lifted);
            }
            Stmt::While(s) => {
                strip(&mut s.body, lifted);
                strip(&mut s.orelse, lifted);
            }
            Stmt::With(s) => strip(&mut s.body, lifted),
            Stmt::Try(s) => {
                strip(&mut s.body, lifted);
                for handler in &mut s.handlers {
                    let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
                    strip(&mut h.body, lifted);
                }
                strip(&mut s.orelse, lifted);
                strip(&mut s.finalbody, lifted);
            }
            Stmt::Match(s) => {
                for case in &mut s.cases {
                    strip(&mut case.body, lifted);
                }
            }
            _ => {}
        }
    }
}

impl Plan {
    /// The lift of unit `index`, when it is one.
    pub(super) fn lift(&self, index: usize) -> Option<&Lift> {
        index.checked_sub(self.top).and_then(|at| self.lifts.get(at))
    }

    /// Whether unit `index` is a top-level statement, or a lift whose
    /// statement runs in CPython (split): anything else is part of a
    /// compiled statement.
    pub(super) fn live(&self, index: usize) -> bool {
        match self.lift(index) {
            Some(lift) => self.fallback.contains_key(&lift.owner) || self.is_attached(index),
            None => true,
        }
    }

    /// Whether unit `index` is a method running in CPython on its compiled
    /// class (`attach`): a lift falling back whose class is compiled.
    pub(super) fn is_attached(&self, index: usize) -> bool {
        self.lift(index).is_some_and(|lift| {
            lift.attach.is_some()
                && self.fallback.contains_key(&index)
                && !self.fallback.contains_key(&lift.owner)
        })
    }

    /// The lifts of top-level statement `owner`, as unit indices.
    pub(super) fn lifts_of(&self, owner: usize) -> impl Iterator<Item = usize> + '_ {
        self.lifts
            .iter()
            .enumerate()
            .filter(move |(_, lift)| lift.owner == owner)
            .map(move |(at, _)| self.top + at)
    }

    /// Whether top-level statement `index` runs in CPython with its `def`s
    /// lifted.
    pub(super) fn is_split(&self, index: usize) -> bool {
        index < self.top
            && self.fallback.contains_key(&index)
            && self.lifts_of(index).next().is_some()
    }

    /// Drops the fallback of every lift whose statement is compiled again
    /// (the retry of `with_fallback`).
    pub(super) fn prune_lifts(&mut self) {
        let dead: Vec<usize> = self
            .fallback
            .keys()
            .copied()
            .filter(|index| {
                self.lift(*index).is_some_and(|lift| {
                    !self.fallback.contains_key(&lift.owner) && lift.attach.is_none()
                })
            })
            .collect();
        for index in dead {
            self.fallback.remove(&index);
        }
    }

    /// The CPython source of split statement `index`: the statement with
    /// each compiled lift's body replaced by its forwarder (padded, so
    /// every line keeps its number).
    pub(super) fn split_source(&self, index: usize) -> String {
        let range = &self.stmts[index].range;
        let src = &self.source;
        let mut out = String::new();
        let mut cursor = range.start as usize;
        for unit in self.lifts_of(index) {
            if self.fallback.contains_key(&unit) {
                continue;
            }
            let lift = self.lift(unit).expect("a lift");
            let (start, end) = (lift.body.start as usize, lift.body.end as usize);
            out.push_str(&src[cursor..start]);
            out.push_str(&lift.forwarder());
            out.push_str(&"\n".repeat(src[start..end].matches('\n').count()));
            cursor = end;
        }
        out.push_str(&src[cursor..range.end as usize]);
        out
    }
}
