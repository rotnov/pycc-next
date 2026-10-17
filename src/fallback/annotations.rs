//! Erasing annotations pycc cannot lower, in a module whose annotations
//! CPython never evaluates (#12).
//!
//! Under `from __future__ import annotations` (PEP 563) an annotation is a
//! string CPython stores and never runs, so a program's behaviour does not
//! depend on it. Real packages write annotations only a type checker
//! resolves: an alias imported under `if TYPE_CHECKING:` (`pos: Pos`), a
//! bare `Final`, `re.Match[str]`, `IO[bytes]`. Falling back the whole
//! function for one of those would run it in CPython; instead the fallback
//! loop erases the annotation the diagnostic points at, and the function
//! compiles as if it had never been annotated there (an unannotated
//! parameter, return or variable, which non-strict mode types as it does
//! any other). A module without the `__future__` import evaluates its
//! annotations when a `def` runs, so its annotations are never erased.
//!
//! The erasure keeps the annotation's line breaks (as `\` continuations),
//! so every later line keeps its number.

use super::*;
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::Expr;

/// One annotation the loop may erase.
#[derive(Debug, Clone)]
pub(super) struct Site {
    /// The annotation expression.
    annotation: Range<u32>,
    /// What erasing it removes: the `: T` of a parameter or a variable, the
    /// `-> T` of a return.
    erase: Range<u32>,
    /// The innermost `def` whose signature holds it, if any.
    def: Option<Range<u32>>,
    /// Every bare name the annotation reads, with its range.
    names: Vec<(Range<u32>, String)>,
    /// An annotated assignment's target, which a diagnostic about the
    /// annotation may point at instead (`self.x: T = v`).
    target: Option<Range<u32>>,
}

/// The bare names `annotation` reads.
fn names_in(annotation: &Expr) -> Vec<(Range<u32>, String)> {
    struct Names(Vec<(Range<u32>, String)>);
    impl<'a> Visitor<'a> for Names {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(name) = expr {
                self.0
                    .push((pycc_ast::expr_range(expr), name.id.as_str().to_string()));
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut names = Names(Vec::new());
    names.visit_expr(annotation);
    names.0
}

/// Whether `body` starts (after its docstring) with a
/// `from __future__ import annotations`.
pub(super) fn defers_annotations(body: &[Stmt]) -> bool {
    let start = usize::from(matches!(body.first(), Some(Stmt::Expr(e))
        if e.value.is_string_literal_expr()));
    body[start.min(body.len())..]
        .iter()
        .map_while(|stmt| match stmt {
            Stmt::ImportFrom(import)
                if import.module.as_ref().map(|m| m.as_str()) == Some("__future__") =>
            {
                Some(import)
            }
            _ => None,
        })
        .any(|import| import.names.iter().any(|alias| alias.name.as_str() == "annotations"))
}

/// Every parameter, return and annotated-assignment annotation in `body`,
/// at any depth.
pub(super) fn sites(source: &str, body: &[Stmt]) -> Vec<Site> {
    let mut collector = Sites {
        source,
        sites: Vec::new(),
    };
    for stmt in body {
        collector.visit_stmt(stmt);
    }
    collector.sites
}

/// Where the annotation spanning `annotation` ends in `source`, its closing
/// quotes included: a string annotation (`x: "P"`) is unquoted when the
/// module parses, its expression keeping the span of the text inside the
/// quotes (`pycc_ast::normalize_string_annotations`).
fn quoted_end(source: &str, annotation: &Range<u32>) -> u32 {
    let bytes = source.as_bytes();
    let (start, end) = (annotation.start as usize, annotation.end as usize);
    let Some(&quote) = start.checked_sub(1).and_then(|i| bytes.get(i)) else {
        return annotation.end;
    };
    if quote != b'"' && quote != b'\'' {
        return annotation.end;
    }
    let opening = bytes[..start].iter().rev().take(3).take_while(|&&b| b == quote).count();
    // A triple-quoted string closes with three quotes, any other with one.
    let closing = if opening == 3 { 3 } else { 1 };
    let closes = bytes
        .get(end..end + closing)
        .is_some_and(|after| after.iter().all(|&b| b == quote));
    if closes { (end + closing) as u32 } else { annotation.end }
}

struct Sites<'s> {
    source: &'s str,
    sites: Vec<Site>,
}

impl<'a> Visitor<'a> for Sites<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                let range = pycc_ast::stmt_range(stmt);
                for param in def.parameters.iter() {
                    if let Some(expr) = param.annotation() {
                        let annotation = pycc_ast::expr_range(expr);
                        self.sites.push(Site {
                            erase: param.name().range.end().to_u32()
                                ..quoted_end(self.source, &annotation),
                            annotation,
                            def: Some(range.clone()),
                            names: names_in(expr),
                            target: None,
                        });
                    }
                }
                if let Some(returns) = def.returns.as_deref() {
                    let annotation = pycc_ast::expr_range(returns);
                    let from = def.parameters.range.end().to_u32();
                    let arrow = self
                        .source
                        .get(from as usize..annotation.start as usize)
                        .and_then(|between| between.find("->"));
                    if let Some(arrow) = arrow {
                        self.sites.push(Site {
                            erase: from + arrow as u32..quoted_end(self.source, &annotation),
                            annotation,
                            def: Some(range),
                            names: names_in(returns),
                            target: None,
                        });
                    }
                }
            }
            // `x: T = v` and `obj.attr: T = v`: erased, `x = v`.
            Stmt::AnnAssign(ann)
                if ann.value.is_some()
                    && (ann.simple || matches!(&*ann.target, Expr::Attribute(_))) =>
            {
                let annotation = pycc_ast::expr_range(&ann.annotation);
                let target = pycc_ast::expr_range(&ann.target);
                self.sites.push(Site {
                    erase: target.end..quoted_end(self.source, &annotation),
                    annotation,
                    def: None,
                    names: names_in(&ann.annotation),
                    target: Some(target),
                });
            }
            _ => {}
        }
        visitor::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, _expr: &'a Expr) {}
}

impl Plan {
    /// Records the erasure of the annotation `failure` points at, returning
    /// whether it did: the annotation holding the failure's span, or, for a
    /// failure on a `def` itself that names an annotation (`type annotation
    /// `Pos` is not supported yet`), every annotation of that `def` naming
    /// it.
    pub(super) fn erase_annotation(&mut self, rendered: Option<&Rendered>, failure: &Failure) -> bool {
        if !self.defers_annotations {
            return false;
        }
        let Some(span) = failure.diagnostic.span.filter(|s| (s.start, s.end) != (0, 0)) else {
            return false;
        };
        let at = match rendered {
            Some(rendered) => rendered.source_or_edit_offset(span.start),
            None => Some(span.start),
        };
        let Some(at) = at else {
            return false;
        };
        let mut chosen: Vec<Range<u32>> = self
            .annotation_sites
            .iter()
            .filter(|site| {
                site.annotation.contains(&at)
                    || site.target.as_ref().is_some_and(|target| target.contains(&at))
            })
            .min_by_key(|site| site.annotation.end - site.annotation.start)
            .map(|site| vec![site.erase.clone()])
            .unwrap_or_default();
        if chosen.is_empty()
            && let Some(name) = annotation_name(&failure.diagnostic.message)
        {
            chosen = self
                .annotation_sites
                .iter()
                .filter(|site| site.def.as_ref().is_some_and(|def| def.start == at))
                .filter(|site| mentions(&self.source[site.annotation.start as usize..site.annotation.end as usize], name))
                .map(|site| site.erase.clone())
                .collect();
        }
        // A diagnostic repeated for one annotation (each use of a signature
        // reports it) is resolved by the one erasure.
        for erase in &chosen {
            let lines = self.source[erase.start as usize..erase.end as usize].matches('\n').count();
            self.erased
                .entry(erase.start)
                .or_insert_with(|| (erase.end, " \\\n".repeat(lines)));
        }
        !chosen.is_empty()
    }
}

/// `X` of a `type annotation `X` ...` message.
fn annotation_name(message: &str) -> Option<&str> {
    let rest = message.strip_prefix("type annotation `")?;
    Some(&rest[..rest.find('`')?])
}

/// Whether the annotation text `text` mentions the identifier `name`.
fn mentions(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        !ident(before) && !ident(after)
    })
}

/// The type aliases a module imports for annotations only: every
/// `from <module> import A, B as C` inside a module-level `if
/// TYPE_CHECKING:` whose module is a file of the program (found from
/// `entry_dir`, relative imports included) that binds the name at its top
/// level to a one-line type expression (`Pos = int`, `Key = Tuple[str,
/// ...]`, `Pos: TypeAlias = int`). Local name -> the expression's source.
///
/// CPython never runs such an import, and under PEP 563 never evaluates an
/// annotation naming it, so pycc may read the alias's definition instead.
pub(super) fn type_checking_aliases(entry_dir: &Path, body: &[Stmt]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for stmt in body {
        let Stmt::If(guard) = stmt else { continue };
        let is_guard = match &*guard.test {
            Expr::Name(name) => name.id.as_str() == "TYPE_CHECKING",
            Expr::Attribute(attr) => attr.attr.as_str() == "TYPE_CHECKING",
            _ => false,
        };
        if !is_guard {
            continue;
        }
        for inner in &guard.body {
            let Stmt::ImportFrom(import) = inner else {
                continue;
            };
            let module = import.module.as_ref().map(|m| m.as_str());
            let Some(path) = module_file(entry_dir, import.level, module) else {
                continue;
            };
            let Some(defined) = std::fs::read(&path)
                .ok()
                .and_then(|bytes| crate::source::decode_python_source(&bytes).ok())
                .map(|text| alias_definitions(&text))
            else {
                continue;
            };
            for alias in &import.names {
                let local = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                if let Some(text) = defined.get(alias.name.as_str()) {
                    out.insert(local.to_string(), text.clone());
                }
            }
        }
    }
    out
}

/// The file of module `module` imported at relative `level` from a module
/// in `entry_dir` (`level` 0: an absolute import of a module beside it).
fn module_file(entry_dir: &Path, level: u32, module: Option<&str>) -> Option<std::path::PathBuf> {
    let mut path = entry_dir.to_path_buf();
    for _ in 1..level.max(1) {
        path = path.parent()?.to_path_buf();
    }
    for part in module?.split('.') {
        path.push(part);
    }
    let file = path.with_extension("py");
    if file.is_file() {
        return Some(file);
    }
    let init = path.join("__init__.py");
    init.is_file().then_some(init)
}

/// A module's top-level one-line type aliases: name -> expression source.
fn alias_definitions(source: &str) -> BTreeMap<String, String> {
    let Ok(module) = pycc_parser::parse_all(source) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for stmt in &module.body {
        let (target, value) = match stmt {
            Stmt::Assign(assign) => match assign.targets.as_slice() {
                [target] => (target, &*assign.value),
                _ => continue,
            },
            Stmt::AnnAssign(ann)
                if matches!(&*ann.annotation, Expr::Name(n) if n.id.as_str() == "TypeAlias") =>
            {
                match ann.value.as_deref() {
                    Some(value) => (&*ann.target, value),
                    None => continue,
                }
            }
            _ => continue,
        };
        let Expr::Name(name) = target else { continue };
        if !is_type_expression(value) {
            continue;
        }
        let range = pycc_ast::expr_range(value);
        let text = &source[range.start as usize..range.end as usize];
        if !text.contains('\n') {
            out.insert(name.id.as_str().to_string(), text.to_string());
        }
    }
    out
}

/// Whether `expr` has the shape of a type expression: a name, an attribute
/// chain, a subscript of one, a `|` union of those, `None` or `...`.
fn is_type_expression(expr: &Expr) -> bool {
    match expr {
        Expr::Name(_) | Expr::NoneLiteral(_) | Expr::EllipsisLiteral(_) => true,
        Expr::Attribute(attr) => is_type_expression(&attr.value),
        Expr::Subscript(sub) => is_type_expression(&sub.value) && is_type_expression(&sub.slice),
        Expr::Tuple(tuple) => tuple.elts.iter().all(is_type_expression),
        Expr::List(list) => list.elts.iter().all(is_type_expression),
        Expr::BinOp(op) => {
            matches!(op.op, pycc_ast::Operator::BitOr)
                && is_type_expression(&op.left)
                && is_type_expression(&op.right)
        }
        _ => false,
    }
}

impl Plan {
    /// Records the substitution of every annotation's read of an alias
    /// [`type_checking_aliases`] finds by the alias's definition, in a
    /// module whose annotations are never evaluated.
    pub(super) fn substitute_aliases(&mut self) {
        if !self.defers_annotations {
            return;
        }
        let Some(dir) = self.entry_dir.clone() else {
            return;
        };
        let aliases = type_checking_aliases(&dir, &self.ast);
        if aliases.is_empty() {
            return;
        }
        for site in &self.annotation_sites {
            for (range, name) in &site.names {
                if let Some(text) = aliases.get(name) {
                    self.substituted
                        .insert(range.start, (range.end, format!("({text})")));
                }
            }
        }
    }
}
