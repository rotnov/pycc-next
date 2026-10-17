//! Nested functions and lambdas in compiled code (v0.8 accept 2).
//!
//! A nested `def` (a decorated wrapper, a callback handed to a library, a
//! recursive helper) or a `lambda` that pycc cannot compile as a value no
//! longer sends its whole enclosing function to CPython: only the nested
//! statement or expression runs in CPython, the enclosing function stays
//! compiled. CPython creates the function object, so its decorators,
//! defaults (evaluated when the `def` runs), annotations, `__name__`,
//! `__qualname__` (`outer.<locals>.inner`), `__wrapped__`, `__dict__` and
//! signature are CPython's own.
//!
//! **The closure.** The nested function reads the enclosing function's
//! variables through real CPython cells, so late binding, `nonlocal` and
//! cells shared between sibling closures behave as in CPython. A captured
//! variable the enclosing function never rebinds after the nested
//! statement can run (a parameter it never assigns, a variable one
//! unconditional top-level assignment before it binds) gets a fresh cell
//! holding its value where the function is made. Any other captured
//! variable lives in a cell for the whole call (`__pycc_cell_x`, made at
//! the function's start): every read, store and `del` of it in the
//! compiled function becomes one of the cell's `cell_contents`.
//!
//! **The rewrite.** At module start the fallback preamble `exec`s, per
//! nested statement, a *scaffold*: a function named like the enclosing one
//! whose free variables are the captured names and whose body is the nested
//! statement, as written (padded so its lines are the source's), followed by
//! `return` of what it made. Its code object, re-qualified so the nested
//! function's `__qualname__` is the enclosing one's, is kept in the
//! namespace (`__pycc_nestN__`). Compiled code replaces the statement with
//! `name = nested(spec, (cell, ...))`, which builds the scaffold over those
//! cells and runs it, exactly where the `def` or `lambda` ran.

use super::*;
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, ExprContext, StmtFunctionDef};

mod body;
mod cells;
mod scopes;

use cells::{CellCheck, CellRewriter};
use scopes::*;

/// The cell of a captured variable, in rewritten compiled code.
pub(super) const CELL_PREFIX: &str = "__pycc_cell_";

/// The preamble helpers' module-namespace names.
const NESTED_FN: &str = "__pycc_fallback_nested";
const CELL_FN: &str = "__pycc_fallback_cell";
const SPEC_FN: &str = "__pycc_fallback_nestspec";

/// One nested `def` or `lambda` of a compiled function that CPython runs.
#[derive(Debug, Clone)]
pub(in crate::fallback) struct Nested {
    /// The compiled unit (`Plan::stmts` index) holding it.
    pub(super) unit: usize,
    /// The source text CPython runs: the `def` with its decorators (from
    /// the start of its first line) or the `lambda` expression.
    pub(super) range: Range<u32>,
    /// The name a nested `def` binds; `None` for a `lambda`.
    pub(super) def_name: Option<String>,
    /// The enclosing function's `def` range, `None` at module level.
    pub(super) outer: Option<Range<u32>>,
    /// The enclosing function's `__qualname__` (`C.m`, `f`; empty at
    /// module level).
    pub(super) outer_qualname: String,
    /// The enclosing function's own name (the scaffold's `co_name`).
    pub(super) outer_name: String,
    /// The captured variables, sorted: the order of the cells passed.
    pub(super) captures: Vec<String>,
    /// The captured variables that live in a cell for the whole call.
    pub(super) celled: BTreeSet<String>,
    /// Why it runs in CPython, for the report.
    pub(super) reason: String,
    /// The module globals its text reads.
    pub(super) mentions: BTreeSet<String>,
    /// A nested `def`'s compiled body (`body`), when it has one.
    pub(super) body: Option<body::Body>,
}

impl Nested {
    /// Its own `__qualname__`.
    pub(super) fn qualname(&self) -> String {
        let name = self.def_name.as_deref().unwrap_or("<lambda>");
        if self.outer_qualname.is_empty() {
            name.to_string()
        } else {
            format!("{}.<locals>.{name}", self.outer_qualname)
        }
    }
}

impl Plan {
    /// Records a nested `def` or `lambda` a `C0001` points at as a nested
    /// statement CPython runs, returning whether it did. Anything it cannot
    /// rewrite exactly is left to fall back with its unit.
    pub(super) fn nest(&mut self, rendered: Option<&Rendered>, failure: &Failure) -> bool {
        let message = failure.diagnostic.message.as_str();
        let unsupported = failure.diagnostic.code == "C0001"
            && (message.contains("a `def` nested inside a function or block body")
                || message.starts_with("expression kind not supported yet: a `lambda`"));
        // A nested function pycc compiled as a function value that then
        // meets a CPython object it cannot cross (`(wrapper, getter)` with
        // `wrapper` already in CPython): the first one of the enclosing
        // function runs in CPython too.
        let value = message.contains("`Callable[");
        // Any other failure inside a `def` nested in a compiled function:
        // that `def` is the smallest enclosing function, so it alone runs
        // in CPython (its body compiled on its own, where it may compile).
        let inner = !unsupported && !value;
        let Some(at) = self.source_start(rendered, failure) else {
            return false;
        };
        let Some(unit) = self.owner_of(rendered, failure) else {
            return false;
        };
        if self.fallback.contains_key(&unit)
            || self
                .nested
                .iter()
                .any(|n| n.unit == unit && n.range.contains(&at))
        {
            return false;
        }
        // A `lambda` inside a compiled `lambda` body runs with that body.
        if self
            .body_owner(unit)
            .is_some_and(|(_, owner)| owner.body.as_ref().is_some_and(|b| b.lambda.is_some()))
        {
            return false;
        }
        let qualprefix = match (self.lift(unit), self.body_owner(unit)) {
            (Some(lift), _) => lift.qualname.clone(),
            (None, Some((_, owner))) => owner.qualname(),
            (None, None) => String::new(),
        };
        let mut finder = Finder {
            at,
            prefix: qualprefix,
            lifted: self.lift(unit).is_some() || self.body_owner(unit).is_some(),
            outer: Vec::new(),
            class: Vec::new(),
            found: None,
            enclosing: None,
        };
        finder.visit_stmt(&self.ast[unit]);
        let (outer, node) = match finder.found {
            Some(found) if unsupported => found,
            // Only one used as a value (a native closure only called stays
            // compiled, its failure resolved in the enclosing function), and
            // not one holding a `def` or `lambda` of its own: that one's
            // frame would be gone, a compiled closure of the compiled body.
            Some((Some(outer), Node::Def(def)))
                if inner
                    && first_nested(def, &[]).is_none()
                    && value_defs(outer.def).iter().any(|d| d.range == def.range) =>
            {
                (Some(outer), Node::Def(def))
            }
            None if value => {
                let Some(outer) = finder.enclosing else {
                    return false;
                };
                let nested: Vec<Range<u32>> = self
                    .nested
                    .iter()
                    .filter(|n| n.unit == unit)
                    .map(|n| n.range.clone())
                    .collect();
                let Some(node) = first_nested(outer.def, &nested) else {
                    return false;
                };
                (Some(outer), node)
            }
            _ => return false,
        };
        let reason = describe(&failure.diagnostic);
        let siblings: Vec<StmtFunctionDef> = outer
            .as_ref()
            .map(|outer| value_defs(outer.def).into_iter().cloned().collect())
            .unwrap_or_default();
        let outer_def = outer
            .as_ref()
            .map(|outer| (outer.def.clone(), outer.qualname.clone()));
        let node_def = match &node {
            Node::Def(def) => Some(Ok((*def).clone())),
            Node::Lambda(expr) => Some(Err((*expr).clone())),
        };
        let Some(nested) = self.analyze(unit, outer, node, reason) else {
            return false;
        };
        self.nested.push(nested);
        match node_def {
            Some(Ok(def)) => self.add_body(self.nested.len() - 1, &def),
            Some(Err(lambda)) => self.add_lambda_body(self.nested.len() - 1, &lambda),
            None => {}
        }
        // A sibling `def` used as a value would cross into CPython as a
        // compiled function value, which has no `__name__`, defaults or
        // `__dict__`: once the function has a nested statement CPython
        // runs, each of them is one too.
        if let Some((def, qualname)) = outer_def {
            for sibling in &siblings {
                let start = sibling.range.end().to_u32();
                if self
                    .nested
                    .iter()
                    .any(|n| n.unit == unit && n.range.end == start)
                {
                    continue;
                }
                let outer = OuterRef {
                    def: &def,
                    qualname: qualname.clone(),
                };
                let reason =
                    "a function value made where a nested function runs in CPython".to_string();
                if let Some(nested) = self.analyze(unit, Some(outer), Node::Def(sibling), reason) {
                    self.nested.push(nested);
                    self.add_body(self.nested.len() - 1, sibling);
                }
            }
        }
        // Every variable a nested statement of the same function keeps in
        // a cell stays there for all of them.
        self.share_cells();
        true
    }

    /// The live nested statements: those of a compiled unit.
    pub(super) fn live_nested(&self) -> impl Iterator<Item = (usize, &Nested)> {
        self.nested
            .iter()
            .enumerate()
            .filter(|(_, n)| !self.fallback.contains_key(&n.unit) && self.live(n.unit))
    }

    /// The module globals the live nested statements read.
    pub(super) fn nested_mentions(&self) -> BTreeSet<String> {
        self.live_nested()
            .flat_map(|(_, n)| n.mentions.iter().cloned())
            .collect()
    }

    /// Makes every nested statement of one enclosing function agree on the
    /// variables kept in a cell.
    fn share_cells(&mut self) {
        let mut by_outer: BTreeMap<(usize, u32), BTreeSet<String>> = BTreeMap::new();
        for n in &self.nested {
            if let Some(outer) = &n.outer {
                by_outer
                    .entry((n.unit, outer.start))
                    .or_default()
                    .extend(n.celled.iter().cloned());
            }
        }
        for n in &mut self.nested {
            if let Some(outer) = &n.outer
                && let Some(celled) = by_outer.get(&(n.unit, outer.start))
            {
                n.celled = n
                    .captures
                    .iter()
                    .filter(|c| celled.contains(*c))
                    .cloned()
                    .collect();
            }
        }
    }

    fn analyze(
        &self,
        unit: usize,
        outer: Option<OuterRef<'_>>,
        node: Node<'_>,
        reason: String,
    ) -> Option<Nested> {
        let src = &self.source;
        let (range, def_name, header, body_free) = match node {
            Node::Def(def) => {
                // `super()` and `__class__` read the method's own cell.
                if uses_class_cell(def) {
                    return None;
                }
                let start = def
                    .decorator_list
                    .first()
                    .map_or(def.range.start().to_u32(), |d| d.range.start().to_u32());
                let line_start = src[..start as usize].rfind('\n').map_or(0, |i| i + 1) as u32;
                // The `def` must start its own line(s).
                if !src[line_start as usize..start as usize].trim().is_empty() {
                    return None;
                }
                let mut header = BTreeSet::new();
                for decorator in &def.decorator_list {
                    header.extend(expr_free(&decorator.expression));
                }
                for param in def.parameters.iter() {
                    if let Some(default) = param.default() {
                        header.extend(expr_free(default));
                    }
                    if let Some(annotation) = param.annotation() {
                        header.extend(expr_free(annotation));
                    }
                }
                if let Some(returns) = &def.returns {
                    header.extend(expr_free(returns));
                }
                let free = function_free(&def.parameters, &def.body);
                (
                    line_start..def.range.end().to_u32(),
                    Some(def.name.as_str().to_string()),
                    header,
                    free,
                )
            }
            Node::Lambda(expr) => {
                let Expr::Lambda(lambda) = expr else {
                    return None;
                };
                let mut header = BTreeSet::new();
                let mut free = BTreeSet::new();
                match &lambda.parameters {
                    Some(parameters) => {
                        for param in parameters.iter() {
                            if let Some(default) = param.default() {
                                header.extend(expr_free(default));
                            }
                        }
                        free.extend(lambda_free(parameters, &lambda.body));
                    }
                    None => free.extend(expr_free(&lambda.body)),
                }
                (
                    lambda.range.start().to_u32()..lambda.range.end().to_u32(),
                    None,
                    header,
                    free,
                )
            }
        };
        // A `lambda` reading `__class__` (or calling `super()`) reads the
        // enclosing method's `__class__` cell.
        if body_free.contains("__class__") {
            return None;
        }
        let referenced: BTreeSet<String> = header.union(&body_free).cloned().collect();
        // A statement nested in a compiled body reads that body's own
        // captured variables through cells this rewrite cannot reach.
        if let Some((_, owner)) = self.body_owner(unit)
            && owner.captures.iter().any(|c| referenced.contains(c))
        {
            return None;
        }
        let (captures, celled, outer_range, outer_name) = match &outer {
            Some(outer) => {
                let locals = outer.locals();
                let mut captures: Vec<String> = referenced
                    .iter()
                    .filter(|name| locals.contains(*name))
                    .cloned()
                    .collect();
                // The `def`'s own name, read by its own body (recursion),
                // is bound by the statement itself.
                captures.sort();
                let statement_index = outer.top_index(range.start);
                let mut celled: BTreeSet<String> = BTreeSet::new();
                for name in &captures {
                    if !outer.stable(name, statement_index, def_name.as_deref()) {
                        celled.insert(name.to_string());
                    }
                }
                if !outer.can_cell(&celled, &self.source) {
                    return None;
                }
                (
                    captures,
                    celled,
                    Some(outer.def.range.start().to_u32()..outer.def.range.end().to_u32()),
                    outer.def.name.as_str().to_string(),
                )
            }
            None => (Vec::new(), BTreeSet::new(), None, String::new()),
        };
        let outer_locals = outer.as_ref().map(OuterRef::locals).unwrap_or_default();
        let mentions = referenced
            .into_iter()
            .filter(|name| !outer_locals.contains(name))
            .collect();
        Some(Nested {
            unit,
            range,
            def_name,
            outer: outer_range,
            outer_qualname: outer
                .as_ref()
                .map(|o| o.qualname.clone())
                .unwrap_or_default(),
            outer_name,
            captures,
            celled,
            reason,
            mentions,
            body: None,
        })
    }

    /// The preamble text defining the helpers and every live nested
    /// statement's scaffold, or empty when there is none.
    pub(super) fn nested_preamble(&self, chunks: &mut Vec<render::CodeChunk>) -> String {
        let live: Vec<(usize, &Nested)> = self.live_nested().collect();
        if live.is_empty() {
            return String::new();
        }
        let helpers = format!(
            "import types as __pycc_fallback_types\n\
             def __pycc_fallback_requal(code, old, new):\n\
             \x20   consts = tuple(__pycc_fallback_requal(c, old, new) if isinstance(c, __pycc_fallback_types.CodeType) else c for c in code.co_consts)\n\
             \x20   q = code.co_qualname\n\
             \x20   if q == old:\n\
             \x20       q = new or code.co_name\n\
             \x20   elif q.startswith(old + '.<locals>.'):\n\
             \x20       q = (new + '.<locals>.' if new else '') + q[len(old) + 10:]\n\
             \x20   return code.replace(co_consts=consts, co_qualname=q)\n\
             def {SPEC_FN}(make, names, old, new):\n\
             \x20   code = __pycc_fallback_requal(make().__code__, old, new)\n\
             \x20   return code, tuple(names.index(n) for n in code.co_freevars)\n\
             def {NESTED_FN}(spec, cells):\n\
             \x20   code, order = spec\n\
             \x20   return __pycc_fallback_types.FunctionType(code, globals(), code.co_name, None, tuple([cells[i] for i in order]) or None)(cells)\n\
             {CELL_FN} = __pycc_fallback_types.CellType\n\
"
        );
        let mut out = format!(
            "; {BUILTINS}.exec({BUILTINS}.compile({}, \"<pycc fallback>\", \"exec\"), {NS})",
            py_str(&helpers)
        );
        for (id, nested) in live {
            out.push_str("; ");
            out.push_str(&self.exec_text(chunks, &self.scaffold(id, nested)));
        }
        out
    }

    /// The scaffold source of nested statement `id` (see the module doc).
    fn scaffold(&self, id: usize, nested: &Nested) -> String {
        let src = &self.source;
        let make = format!("__pycc_nest{id}_make");
        let mut name = if nested.outer_name.is_empty() {
            "__pycc_module".to_string()
        } else {
            nested.outer_name.clone()
        };
        if nested.captures.contains(&name) {
            name = format!("__pycc_scaffold{id}");
        }
        let mut header = vec![format!("def {make}():")];
        if !nested.captures.is_empty() {
            header.push(format!(" {} = None", nested.captures.join(" = ")));
        }
        header.push(format!(" def {name}({}):", body::CELLS_PARAM));
        let start = nested.range.start as usize;
        let line = src[..start].matches('\n').count();
        let pad = line.saturating_sub(header.len());
        let mut text = "\n".repeat(pad);
        text.push_str(&header.join("\n"));
        text.push('\n');
        let body = &src[start..nested.range.end as usize];
        match &nested.def_name {
            Some(def_name) => {
                match self.compiled_body(nested) {
                    // The compiled body's forwarder replaces the body.
                    Some(compiled) => {
                        text.push_str(&src[start..compiled.start as usize]);
                        text.push_str(&self.forwarder(compiled));
                    }
                    None => text.push_str(body),
                }
                let indent: String = body
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                text.push_str(&format!("\n{indent}return {def_name}\n"));
            }
            None => {
                let body = match self.compiled_body(nested) {
                    Some(compiled) => format!(
                        "{}{}",
                        &src[start..compiled.start as usize],
                        self.forwarder(compiled)
                    ),
                    None => body.to_string(),
                };
                text.push_str(&format!("  return ({body})\n"));
            }
        }
        text.push_str(&format!(" return {name}\n"));
        let old = format!("{make}.<locals>.{name}");
        let names: Vec<String> = nested.captures.iter().map(|c| py_str(c)).collect();
        text.push_str(&format!(
            "__pycc_nest{id}__ = {SPEC_FN}({make}, ({}), {}, {})\ndel {make}\n",
            if names.is_empty() {
                String::new()
            } else {
                format!("{},", names.join(", "))
            },
            py_str(&old),
            py_str(&nested.outer_qualname),
        ));
        text
    }

    /// The edits of compiled unit `index` for its live nested statements:
    /// each statement replaced by its call, every celled variable read and
    /// stored through its cell, and the cells made at the function's start.
    pub(super) fn nested_edits(&self, index: usize) -> Vec<names::Edit> {
        // A compiled body's own header and cells come first.
        let mut edits = self.body_edits(index);
        let nested: Vec<(usize, &Nested)> = self
            .live_nested()
            .filter(|(_, n)| n.unit == index)
            .collect();
        if nested.is_empty() {
            return edits;
        }
        let src = &self.source;
        let edit = |range: Range<u32>, text: String| names::Edit {
            range,
            text,
            mirror: None,
        };
        let cell = |name: &str| format!("{CELL_PREFIX}{name}");
        for (id, n) in &nested {
            let cells: Vec<String> = n
                .captures
                .iter()
                .map(|c| {
                    if n.celled.contains(c) {
                        cell(c)
                    } else {
                        format!("{NS}[\"{CELL_FN}\"]({c})")
                    }
                })
                .collect();
            let call = format!(
                "{NS}[\"{NESTED_FN}\"]({}{NS}[\"__pycc_nest{id}__\"], ({}))",
                "\n".repeat(
                    src[n.range.start as usize..n.range.end as usize]
                        .matches('\n')
                        .count()
                ),
                if cells.is_empty() {
                    String::new()
                } else {
                    format!("{},", cells.join(", "))
                }
            );
            let text = match &n.def_name {
                Some(name) => {
                    let indent: String = src[n.range.start as usize..]
                        .chars()
                        .take_while(|c| *c == ' ' || *c == '\t')
                        .collect();
                    let target = if n.celled.contains(name) {
                        format!("{}.cell_contents", cell(name))
                    } else {
                        name.clone()
                    };
                    format!("{indent}{target} = {call}")
                }
                None => call,
            };
            edits.push(edit(n.range.clone(), text));
        }
        // The cells, per enclosing function.
        let mut outers: BTreeMap<u32, (Range<u32>, BTreeSet<String>)> = BTreeMap::new();
        for (_, n) in &nested {
            if let Some(outer) = &n.outer {
                outers
                    .entry(outer.start)
                    .or_insert_with(|| (outer.clone(), BTreeSet::new()))
                    .1
                    .extend(n.celled.iter().cloned());
            }
        }
        let skip: Vec<Range<u32>> = nested.iter().map(|(_, n)| n.range.clone()).collect();
        for (_, (outer_range, celled)) in outers {
            if celled.is_empty() {
                continue;
            }
            let Some(def) = find_def(&self.ast[index], outer_range.start) else {
                continue;
            };
            let params = parameter_names(&def.parameters);
            // The cells are made first thing in the body, after its
            // docstring.
            let first = def
                .body
                .iter()
                .find(|stmt| !is_docstring(stmt))
                .or(def.body.last())
                .map(|stmt| pycc_ast::stmt_range(stmt).start);
            let Some(first) = first else { continue };
            let line_start = src[..first as usize].rfind('\n').map_or(0, |i| i + 1) as u32;
            let indent = &src[line_start as usize..first as usize];
            let inits: Vec<String> = celled
                .iter()
                .map(|name| {
                    let value = if params.contains(name) {
                        name.as_str()
                    } else {
                        ""
                    };
                    format!("{} = {NS}[\"{CELL_FN}\"]({value})", cell(name))
                })
                .collect();
            edits.push(edit(
                line_start..line_start,
                format!("{indent}{}\n", inits.join("; ")),
            ));
            let mut rewriter = CellRewriter {
                celled: &celled,
                source: src,
                skip: &skip,
                scopes: vec![BTreeSet::new()],
                edits: Vec::new(),
                cell: &cell,
            };
            for stmt in &def.body {
                rewriter.visit_stmt(stmt);
            }
            edits.extend(rewriter.edits);
        }
        edits
    }
}

/// What [`Finder`] found the failure in.
enum Node<'a> {
    Def(&'a StmtFunctionDef),
    /// An `Expr::Lambda`.
    Lambda(&'a Expr),
}

/// The enclosing compiled function of a nested statement.
struct OuterRef<'a> {
    def: &'a StmtFunctionDef,
    qualname: String,
}

impl OuterRef<'_> {
    /// Its local variables: its parameters and what its body binds, but
    /// what it declares `global`.
    fn locals(&self) -> BTreeSet<String> {
        let globals = names::declared_globals(&self.def.body);
        let mut locals = names::bound_names(&self.def.body);
        locals.extend(parameter_names(&self.def.parameters));
        locals.retain(|name| !globals.contains(name));
        locals
    }

    /// The index of the top-level statement of the body holding `at`.
    fn top_index(&self, at: u32) -> usize {
        self.def
            .body
            .iter()
            .position(|stmt| pycc_ast::stmt_range(stmt).contains(&at))
            .unwrap_or(self.def.body.len())
    }

    /// Whether `name` keeps one value from before the top-level statement
    /// `index` on: a parameter the body never binds, or a variable bound
    /// once, by an unconditional top-level assignment before it.
    fn stable(&self, name: &str, index: usize, def_name: Option<&str>) -> bool {
        if def_name == Some(name) || nonlocal_anywhere(&self.def.body, name) {
            return false;
        }
        let sites = binding_sites(&self.def.body, name);
        if parameter_names(&self.def.parameters).contains(name) {
            return sites == 0;
        }
        sites == 1
            && self.def.body[..index.min(self.def.body.len())]
                .iter()
                .any(|stmt| match stmt {
                    Stmt::Assign(a) => {
                        a.targets.len() == 1
                            && matches!(&a.targets[0], Expr::Name(n) if n.id.as_str() == name)
                    }
                    Stmt::AnnAssign(a) => {
                        a.value.is_some()
                            && matches!(a.target.as_ref(), Expr::Name(n) if n.id.as_str() == name)
                    }
                    _ => false,
                })
    }

    /// Whether every binding of every name in `celled` in the body is one
    /// [`CellRewriter`] rewrites (a plain, augmented or annotated
    /// assignment, a tuple target, `del`, the nested `def` itself).
    fn can_cell(&self, celled: &BTreeSet<String>, source: &str) -> bool {
        if celled.is_empty() {
            return true;
        }
        let mut check = CellCheck {
            celled,
            source,
            ok: true,
        };
        for stmt in &self.def.body {
            check.visit_stmt(stmt);
        }
        check.ok
    }
}

/// Finds the outermost nested `def` or `lambda` holding `at` inside a
/// compiled function of the unit, with that function.
struct Finder<'a> {
    at: u32,
    /// The qualname prefix of the unit (a lift's `C.m`).
    prefix: String,
    /// Whether the unit is a lift, whose top `def` is the function itself.
    lifted: bool,
    outer: Vec<&'a StmtFunctionDef>,
    class: Vec<String>,
    found: Option<(Option<OuterRef<'a>>, Node<'a>)>,
    /// The innermost compiled function holding `at`.
    enclosing: Option<OuterRef<'a>>,
}

impl<'a> Finder<'a> {
    fn qualname(&self, def: &StmtFunctionDef) -> String {
        if self.lifted && self.outer.len() <= 1 {
            return self.prefix.clone();
        }
        let mut parts = self.class.clone();
        parts.push(def.name.as_str().to_string());
        parts.join(".")
    }

    fn contains(&self, start: u32, end: u32) -> bool {
        (start..end).contains(&self.at)
    }
}

impl<'a> Visitor<'a> for Finder<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if self.found.is_some() {
            return;
        }
        match stmt {
            Stmt::FunctionDef(def) => {
                let start = def
                    .decorator_list
                    .first()
                    .map_or(def.range.start().to_u32(), |d| d.range.start().to_u32());
                if !self.contains(start, def.range.end().to_u32()) {
                    return;
                }
                if let Some(outer) = self.outer.last() {
                    // A `def` nested in a compiled function.
                    let qualname = self.outer_qualname();
                    self.found = Some((
                        Some(OuterRef {
                            def: outer,
                            qualname,
                        }),
                        Node::Def(def),
                    ));
                    return;
                }
                if def.is_async {
                    return;
                }
                self.outer.push(def);
                self.enclosing = Some(OuterRef {
                    def,
                    qualname: self.outer_qualname(),
                });
                for stmt in &def.body {
                    self.visit_stmt(stmt);
                }
                self.outer.pop();
            }
            Stmt::ClassDef(class) if self.outer.is_empty() => {
                self.class.push(class.name.as_str().to_string());
                for stmt in &class.body {
                    self.visit_stmt(stmt);
                }
                self.class.pop();
            }
            Stmt::ClassDef(_) => {}
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        if self.found.is_some() {
            return;
        }
        if let Expr::Lambda(lambda) = expr
            && self.contains(lambda.range.start().to_u32(), lambda.range.end().to_u32())
        {
            let outer = self.outer.last().map(|outer| OuterRef {
                def: outer,
                qualname: self.outer_qualname(),
            });
            // A lambda in a class body outside any function has no
            // function scope to hold.
            if outer.is_none() && !self.class.is_empty() {
                return;
            }
            self.found = Some((outer, Node::Lambda(expr)));
            return;
        }
        // A `lambda` in a comprehension may read its loop variables, which
        // a scaffold outside it cannot close over: it stays with its unit.
        if matches!(
            expr,
            Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_)
        ) {
            let range = pycc_ast::expr_range(expr);
            if self.contains(range.start, range.end) {
                return;
            }
        }
        visitor::walk_expr(self, expr);
    }
}

impl Finder<'_> {
    fn outer_qualname(&self) -> String {
        match self.outer.last() {
            Some(def) => self.qualname(def),
            None => String::new(),
        }
    }
}

/// The `def`s nested directly in `def`'s body (not inside another one)
/// whose name is read other than as a direct call's callee.
fn value_defs(def: &StmtFunctionDef) -> Vec<&StmtFunctionDef> {
    struct Defs<'a>(Vec<&'a StmtFunctionDef>);
    impl<'a> Visitor<'a> for Defs<'a> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => self.0.push(def),
                Stmt::ClassDef(_) => {}
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, _expr: &'a Expr) {}
    }
    struct ValueUses<'n> {
        name: &'n str,
        found: bool,
    }
    impl<'a> Visitor<'a> for ValueUses<'_> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Call(call) if matches!(&*call.func, Expr::Name(n) if n.id.as_str() == self.name) =>
                {
                    visitor::walk_arguments(self, &call.arguments);
                }
                Expr::Name(n) if n.id.as_str() == self.name && n.ctx == ExprContext::Load => {
                    self.found = true;
                }
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    let mut defs = Defs(Vec::new());
    for stmt in &def.body {
        defs.visit_stmt(stmt);
    }
    defs.0
        .into_iter()
        .filter(|nested| {
            let mut uses = ValueUses {
                name: nested.name.as_str(),
                found: false,
            };
            for stmt in &def.body {
                uses.visit_stmt(stmt);
            }
            uses.found || !nested.decorator_list.is_empty()
        })
        .collect()
}

/// The first nested `def` or `lambda` of `def`'s body (not inside another
/// one) whose range is not in `nested`.
fn first_nested<'a>(def: &'a StmtFunctionDef, nested: &[Range<u32>]) -> Option<Node<'a>> {
    struct First<'a, 'n> {
        nested: &'n [Range<u32>],
        found: Option<Node<'a>>,
    }
    impl<'a> Visitor<'a> for First<'a, '_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if self.found.is_some() {
                return;
            }
            match stmt {
                Stmt::FunctionDef(def) => {
                    let end = def.range.end().to_u32();
                    if !self.nested.iter().any(|range| range.end == end) {
                        self.found = Some(Node::Def(def));
                    }
                }
                Stmt::ClassDef(_) => {}
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            if self.found.is_some() {
                return;
            }
            if let Expr::Lambda(lambda) = expr {
                let start = lambda.range.start().to_u32();
                if !self.nested.iter().any(|range| range.start == start) {
                    self.found = Some(Node::Lambda(expr));
                }
                return;
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut first = First {
        nested,
        found: None,
    };
    for stmt in &def.body {
        first.visit_stmt(stmt);
    }
    first.found
}

/// The `def` starting at `start` in `stmt`, at any depth.
fn find_def(stmt: &Stmt, start: u32) -> Option<&StmtFunctionDef> {
    struct Find<'a> {
        start: u32,
        found: Option<&'a StmtFunctionDef>,
    }
    impl<'a> Visitor<'a> for Find<'a> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if let Stmt::FunctionDef(def) = stmt
                && def.range.start().to_u32() == self.start
            {
                self.found = Some(def);
                return;
            }
            visitor::walk_stmt(self, stmt);
        }
    }
    let mut find = Find { start, found: None };
    find.visit_stmt(stmt);
    find.found
}
