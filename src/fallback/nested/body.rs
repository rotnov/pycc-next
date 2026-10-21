//! The compiled bodies of nested `def`s (`nested`).
//!
//! A nested `def` CPython creates (its decorators, defaults, annotations
//! and signature CPython's own) keeps only a forwarder for a body, `return
//! __pycc_lift_nestN__(__pycc_cells, a, b, args, kwargs)`, passing every
//! parameter positionally (`*args` and `**kwargs` as the tuple and the dict)
//! and the tuple of the cells it closes over, as a lifted method's `def`
//! does (`lift`). pycc compiles the body as the module-level function
//! `__pycc_lift_nestN__`, every parameter and its return an `object`, each
//! captured variable read and written through its cell
//! (`__pycc_cells[i].cell_contents`) and `nonlocal` dropped. The body is a
//! unit of its own (after the lifts), so a body the compiler refuses runs
//! in CPython as written, the enclosing function still compiled. The
//! compiled bodies are defined right after the fallback preamble, before
//! any compiled function can make one of their functions.

use super::super::*;
use super::Nested;
use super::cells::{CellCheck, CellRewriter};
use super::scopes::*;
use pycc_ast::StmtFunctionDef;
use pycc_ast::visitor::Visitor;

/// The compiled body's name prefix: a lift's, so the module publishes it.
const BODY_PREFIX: &str = "__pycc_lift_nest";

/// The scaffold function's parameter holding the tuple of cells.
pub(super) const CELLS_PARAM: &str = "__pycc_cells";

/// A nested `def`'s compiled body.
#[derive(Debug, Clone)]
pub(in crate::fallback) struct Body {
    /// Its unit (`Plan::stmts` index).
    pub(in crate::fallback) unit: usize,
    /// The compiled function's name.
    pub(in crate::fallback) name: String,
    /// The parameters the forwarder passes, in order.
    pub(in crate::fallback) params: Vec<String>,
    /// Where the forwarder replaces the body: the start of the line of its
    /// first statement after the docstring.
    pub(in crate::fallback) start: u32,
    /// The end of the `def`.
    pub(in crate::fallback) end: u32,
    /// For a `lambda`: where it starts (its body, at `start`, is one
    /// expression on its line).
    pub(in crate::fallback) lambda: Option<u32>,
}

impl Plan {
    /// Makes nested statement `id` (a `def`) a compiled body when it can
    /// be one: not `async`, not a generator, with a statement after its
    /// docstring on lines of its own, and every captured variable it
    /// writes (`nonlocal`) bound in ways the body can write through a cell.
    pub(in crate::fallback) fn add_body(&mut self, id: usize, def: &StmtFunctionDef) {
        if def.is_async || pycc_ast::body_yields(&def.body) {
            return;
        }
        let Some(first) = def.body.iter().find(|stmt| !is_docstring(stmt)) else {
            return;
        };
        let first = pycc_ast::stmt_range(first).start;
        let src = &self.source;
        let start = src[..first as usize].rfind('\n').map_or(0, |i| i + 1) as u32;
        let header_end = def
            .body
            .first()
            .map_or(first, |stmt| pycc_ast::stmt_range(stmt).start);
        if start <= def.range.start().to_u32()
            || !src[start as usize..first as usize].trim().is_empty()
            || header_end < def.range.start().to_u32()
        {
            return;
        }
        let nested = &self.nested[id];
        let captures: BTreeSet<String> = nested.captures.iter().cloned().collect();
        let mut check = CellCheck {
            celled: &captures,
            source: src,
            ok: true,
        };
        for stmt in &def.body {
            check.visit_stmt(stmt);
        }
        if !check.ok {
            return;
        }
        let params: Vec<String> = def
            .parameters
            .iter()
            .map(|p| p.name().as_str().to_string())
            .collect();
        self.push_body(
            id,
            Stmt::FunctionDef(def.clone()),
            params,
            start,
            def.range.end().to_u32(),
            None,
        );
    }

    /// [`Self::add_body`] of a `lambda` (`expr`) written on one line, whose
    /// compiled function returns its body.
    pub(in crate::fallback) fn add_lambda_body(&mut self, id: usize, expr: &pycc_ast::Expr) {
        let pycc_ast::Expr::Lambda(lambda) = expr else {
            return;
        };
        let (start, end) = (lambda.range.start().to_u32(), lambda.range.end().to_u32());
        if self.source[start as usize..end as usize].contains('\n') {
            return;
        }
        let params: Vec<String> = lambda
            .parameters
            .iter()
            .flat_map(|parameters| parameters.iter())
            .map(|p| p.name().as_str().to_string())
            .collect();
        // Just after the `:` ending the parameters: the body may be
        // parenthesized (`lambda b: (lambda c: b + c)`), and the
        // parentheses are part of it.
        let params_end = lambda
            .parameters
            .as_ref()
            .map_or(start + "lambda".len() as u32, |p| p.range.end().to_u32());
        let body_expr = pycc_ast::expr_range(&lambda.body).start;
        let Some(colon) = self.source[params_end as usize..body_expr as usize].find(':') else {
            return;
        };
        let body_start = params_end + colon as u32 + 1;
        let analyzed = Stmt::Expr(pycc_ast::StmtExpr {
            node_index: Default::default(),
            range: lambda.range,
            value: Box::new(expr.clone()),
        });
        self.push_body(id, analyzed, params, body_start, end, Some(start));
    }

    /// Records the compiled body unit of nested statement `id`.
    fn push_body(
        &mut self,
        id: usize,
        analyzed: Stmt,
        params: Vec<String>,
        start: u32,
        end: u32,
        lambda: Option<u32>,
    ) {
        let name = format!("{BODY_PREFIX}{id}__");
        let unit = self.stmts.len();
        self.stmts.push(TopStmt {
            range: lambda.unwrap_or(start)..end,
            kind: Kind::Def(name.clone(), false),
            bound: BTreeSet::from([name.clone()]),
            mentions: names::module_references(&analyzed),
            opaque: BTreeSet::from([name.clone()]),
            declares: names::all_global_declarations(&analyzed),
            defaults: false,
            hidden_members: false,
            unpublished_members: false,
            bases: BTreeSet::new(),
        });
        self.ast.push(analyzed);
        let nested = &mut self.nested[id];
        nested.mentions.insert(name.clone());
        nested.body = Some(Body {
            unit,
            name,
            params,
            start,
            end,
            lambda,
        });
    }

    /// The nested statement whose compiled body is unit `index`.
    pub(in crate::fallback) fn body_owner(&self, index: usize) -> Option<(usize, &Nested)> {
        self.nested
            .iter()
            .enumerate()
            .find(|(_, n)| n.body.as_ref().is_some_and(|b| b.unit == index))
    }

    /// The compiled body of nested statement `nested`, when it compiles:
    /// its statement runs, and its own unit does not fall back.
    pub(in crate::fallback) fn compiled_body<'n>(&self, nested: &'n Nested) -> Option<&'n Body> {
        nested
            .body
            .as_ref()
            .filter(|body| !self.fallback.contains_key(&body.unit))
    }

    /// The units of the compiled bodies of the live nested statements, in
    /// order: rendered after the fallback preamble.
    pub(in crate::fallback) fn nested_body_units(&self) -> Vec<usize> {
        self.live_nested()
            .filter_map(|(_, n)| self.compiled_body(n).map(|b| b.unit))
            .collect()
    }

    /// The edits making unit `index`, a nested statement's body, the
    /// compiled function: its header, and each captured variable read and
    /// stored through its cell.
    pub(in crate::fallback) fn body_edits(&self, index: usize) -> Vec<names::Edit> {
        let Some((_, nested)) = self.body_owner(index) else {
            return Vec::new();
        };
        let Some(body) = &nested.body else {
            return Vec::new();
        };
        let params: Vec<String> = std::iter::once(format!("{CELLS_PARAM}: object"))
            .chain(body.params.iter().map(|p| format!("{p}: object")))
            .collect();
        let header = format!("def {}({}) -> object:", body.name, params.join(", "));
        let mut edits = vec![match body.lambda {
            // `lambda a: e` becomes `def f(cells, a) -> object: return e`.
            Some(lambda) => names::Edit {
                range: lambda..body.start,
                text: format!("{header} return"),
                mirror: None,
            },
            None => names::Edit {
                range: body.start..body.start,
                text: format!("{header}\n"),
                mirror: None,
            },
        }];
        let captures: BTreeSet<String> = nested.captures.iter().cloned().collect();
        if captures.is_empty() {
            return edits;
        }
        let index_of = |name: &str| nested.captures.iter().position(|c| c == name).unwrap_or(0);
        let cell = |name: &str| format!("{CELLS_PARAM}[{}]", index_of(name));
        let skip: Vec<Range<u32>> = self
            .nested
            .iter()
            .filter(|n| n.unit == index)
            .map(|n| n.range.clone())
            .collect();
        let mut rewriter = CellRewriter {
            celled: &captures,
            source: &self.source,
            skip: &skip,
            // The body's own parameters and locals shadow a name only its
            // header (a default, a decorator) reads from the enclosing
            // function.
            scopes: vec![BTreeSet::new(), self.body_locals(index)],
            edits: Vec::new(),
            cell: &cell,
        };
        match &self.ast[index] {
            Stmt::FunctionDef(def) => {
                for stmt in &def.body {
                    if pycc_ast::stmt_range(stmt).start >= body.start {
                        rewriter.visit_stmt(stmt);
                    }
                }
            }
            Stmt::Expr(expr) => {
                if let pycc_ast::Expr::Lambda(lambda) = expr.value.as_ref() {
                    rewriter.visit_expr(&lambda.body);
                }
            }
            _ => {}
        }
        edits.extend(rewriter.edits);
        edits
    }

    /// The parameters and the variables of the compiled body unit `index`
    /// (those it binds without declaring them `nonlocal`).
    fn body_locals(&self, index: usize) -> BTreeSet<String> {
        match &self.ast[index] {
            Stmt::FunctionDef(def) => {
                let nonlocals = declared_nonlocals(&def.body);
                let mut own = names::bound_names(&def.body);
                own.extend(parameter_names(&def.parameters));
                own.retain(|name| !nonlocals.contains(name));
                own
            }
            Stmt::Expr(expr) => match expr.value.as_ref() {
                pycc_ast::Expr::Lambda(lambda) => lambda
                    .parameters
                    .as_deref()
                    .map(parameter_names)
                    .unwrap_or_default(),
                _ => BTreeSet::new(),
            },
            _ => BTreeSet::new(),
        }
    }

    /// The forwarder replacing a compiled body in the scaffold's `def`,
    /// padded to the body's lines.
    pub(in crate::fallback) fn forwarder(&self, body: &Body) -> String {
        let src = &self.source;
        if body.lambda.is_some() {
            let args: Vec<&str> = std::iter::once(CELLS_PARAM)
                .chain(body.params.iter().map(String::as_str))
                .collect();
            return format!(" {}({})", body.name, args.join(", "));
        }
        let text = &src[body.start as usize..body.end as usize];
        let indent: String = text
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let args: Vec<&str> = std::iter::once(CELLS_PARAM)
            .chain(body.params.iter().map(String::as_str))
            .collect();
        format!(
            "{indent}return {}({}){}",
            body.name,
            args.join(", "),
            "\n".repeat(text.matches('\n').count())
        )
    }
}
