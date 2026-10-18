//! Source positions of statements (v0.8 accept 3): what a traceback entry
//! and a frame's `f_lineno` report for compiled code.
//!
//! HIR carries no spans, so [`lower_body`](crate::stmt) puts a
//! [`HirStmt::Pos`](crate::HirStmt::Pos) marker before each statement it
//! lowers. The marker holds CPython's location for the instruction that
//! statement fails at in the common case: the statement's main expression
//! (the value of an assignment or `return`, the expression of an expression
//! statement, the test of an `if`/`while`, the iterable of a `for`) or the
//! whole statement (`raise`, `assert`, `x += y`, `del`, `import`). Lines are
//! 1-based and columns are UTF-8 byte offsets, as in `co_positions()`.
//!
//! A statement's marker also names its *sites* ([`position_sites`]): the
//! calls, subscripts, attribute loads and divisions of its own expressions,
//! in pre-order, each with its own position. Codegen matches the operation
//! it is emitting against them by kind and name (the second call of `f` in
//! the statement is the second `f` site), so a failure inside `print(f(x))`
//! is located at `f(x)`, as CPython locates the instruction that failed.
//!
//! The driver installs the text the parsed module's offsets index
//! ([`with_source`]); with none installed (unit tests) no marker is made.

use std::cell::RefCell;

/// A source location: `file` is the program's module index, the rest is a
/// `co_positions()` entry. `col == u32::MAX` means "no column information".
/// `sites` indexes a statement marker's sites ([`position_sites`]), or is
/// `u32::MAX` for none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SrcPos {
    pub file: u32,
    pub line: u32,
    pub end_line: u32,
    pub col: u32,
    pub end_col: u32,
    pub sites: u32,
}

impl SrcPos {
    /// `self` as a plain location, without its sites.
    #[must_use]
    pub fn location(self) -> Self {
        Self {
            sites: u32::MAX,
            ..self
        }
    }
}

/// One site of a statement: what it is (`f` for a call of a function or
/// method named `f`, `.a` for a load of attribute `a`, `[]` for a
/// subscript, `/` for `/`, `//` or `%`) and where.
pub type Site = (String, SrcPos);

/// Every statement's sites, by `SrcPos::sites`: process-wide, appended to
/// as statements are lowered (a statement lowered again on another attempt
/// gets a new entry).
static SITES: std::sync::Mutex<Vec<Vec<Site>>> = std::sync::Mutex::new(Vec::new());

/// The sites table codegen reads (`SrcPos::sites` indexes it).
pub fn position_sites() -> Vec<Vec<Site>> {
    SITES.lock().map(|sites| sites.clone()).unwrap_or_default()
}

struct Source {
    file: u32,
    /// Where the parsed text's offsets map in the file: `(offset in text,
    /// offset in the file, length)` runs, or `None` when the text is the
    /// file itself.
    copied: Option<Vec<(u32, u32, u32)>>,
    /// Byte offset of the start of each line of the file.
    line_starts: Vec<u32>,
}

thread_local! {
    static SOURCE: RefCell<Option<Source>> = const { RefCell::new(None) };
}

/// The display path of each module index [`with_source`] has seen. Module
/// indices are assigned in load order, the same on every attempt of a build
/// (the fallback loop's included), so one process-wide table serves them.
static FILES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The display path of each source file, by `SrcPos::file`: what codegen
/// names a compiled frame's file.
pub fn source_files() -> Vec<String> {
    FILES.lock().map(|files| files.clone()).unwrap_or_default()
}

/// Runs `f` with `file` (the program's index of the module being lowered,
/// shown as `display`, whose file text is `original`) as the source of every marker made. When
/// the parsed text is a rewrite of the file, `copied` maps its offsets back
/// (`(offset in text, offset in the file, length)` runs); a statement
/// outside every run gets no marker.
pub fn with_source<R>(
    file: u32,
    display: &str,
    original: &str,
    copied: Option<Vec<(u32, u32, u32)>>,
    f: impl FnOnce() -> R,
) -> R {
    if let Ok(mut files) = FILES.lock() {
        let index = file as usize;
        if files.len() <= index {
            files.resize(index + 1, String::new());
        }
        files[index] = display.to_string();
    }
    let mut line_starts = vec![0u32];
    line_starts.extend(
        original
            .bytes()
            .enumerate()
            .filter(|&(_, b)| b == b'\n')
            .map(|(i, _)| i as u32 + 1),
    );
    let previous = SOURCE.with(|cell| {
        cell.borrow_mut().replace(Source {
            file,
            copied,
            line_starts,
        })
    });
    let result = f();
    SOURCE.with(|cell| *cell.borrow_mut() = previous);
    result
}

impl Source {
    fn file_offset(&self, at: u32) -> Option<u32> {
        match &self.copied {
            None => Some(at),
            Some(runs) => runs
                .iter()
                .find(|(start, _, len)| (*start..=start + len).contains(&at))
                .map(|(start, source, _)| source + (at - start)),
        }
    }

    fn line_col(&self, offset: u32) -> (u32, u32) {
        let line = self.line_starts.partition_point(|&start| start <= offset);
        (line as u32, offset - self.line_starts[line - 1])
    }

    fn position(&self, start: u32, end: u32) -> Option<SrcPos> {
        if end <= start {
            return None;
        }
        let start = self.file_offset(start)?;
        let end = self.file_offset(end)?;
        if end < start {
            return None;
        }
        let (line, col) = self.line_col(start);
        let (end_line, end_col) = self.line_col(end);
        Some(SrcPos {
            file: self.file,
            line,
            end_line,
            col,
            end_col,
            sites: u32::MAX,
        })
    }
}

/// The position of the byte range `start..end` of the parsed text, or
/// `None` when no source is installed or the range is empty or synthetic.
pub fn position_of(start: u32, end: u32) -> Option<SrcPos> {
    SOURCE.with(|cell| cell.borrow().as_ref().and_then(|s| s.position(start, end)))
}

/// Whether evaluating `expr` cannot fail: a literal or a name.
fn cannot_fail(expr: &pycc_ast::Expr) -> bool {
    use pycc_ast::Expr;
    matches!(
        expr,
        Expr::NumberLiteral(_)
            | Expr::StringLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::EllipsisLiteral(_)
            | Expr::Name(_)
    )
}

/// Collects a statement's sites (module documentation) in pre-order.
struct SiteFinder {
    sites: Vec<Site>,
}

impl SiteFinder {
    fn push(&mut self, key: String, range: pycc_ast::TextRange) {
        if let Some(pos) = position_of(range.start().into(), range.end().into()) {
            self.sites.push((key, pos));
        }
    }
}

impl<'a> pycc_ast::visitor::Visitor<'a> for SiteFinder {
    fn visit_expr(&mut self, expr: &'a pycc_ast::Expr) {
        use pycc_ast::{Expr, Operator, Ranged};
        match expr {
            // A lambda's body is another function's.
            Expr::Lambda(_) => return,
            Expr::Call(call) => {
                let key = match call.func.as_ref() {
                    Expr::Name(name) => Some(name.id.to_string()),
                    Expr::Attribute(attr) => Some(attr.attr.to_string()),
                    _ => None,
                };
                if let Some(key) = key {
                    self.push(key, expr.range());
                }
                // The callee's own attribute is the call's, not a load.
                if let Expr::Attribute(attr) = call.func.as_ref() {
                    self.visit_expr(&attr.value);
                } else {
                    self.visit_expr(&call.func);
                }
                for arg in &call.arguments.args {
                    self.visit_expr(arg);
                }
                for keyword in &call.arguments.keywords {
                    self.visit_expr(&keyword.value);
                }
                return;
            }
            Expr::Subscript(_) => self.push("[]".to_string(), expr.range()),
            Expr::Attribute(attr) => self.push(format!(".{}", attr.attr), expr.range()),
            Expr::BinOp(binop)
                if matches!(binop.op, Operator::Div | Operator::FloorDiv | Operator::Mod) =>
            {
                self.push("/".to_string(), expr.range());
            }
            _ => {}
        }
        pycc_ast::visitor::walk_expr(self, expr);
    }
}

/// Registers the sites of `exprs` and answers their index, or `u32::MAX`
/// when there is none.
fn register_sites(exprs: &[&pycc_ast::Expr]) -> u32 {
    use pycc_ast::visitor::Visitor;
    let mut finder = SiteFinder { sites: Vec::new() };
    for expr in exprs {
        finder.visit_expr(expr);
    }
    if finder.sites.is_empty() {
        return u32::MAX;
    }
    let Ok(mut table) = SITES.lock() else {
        return u32::MAX;
    };
    table.push(finder.sites);
    (table.len() - 1) as u32
}

/// The marker position of `stmt` (see the module documentation).
pub(crate) fn stmt_position(stmt: &pycc_ast::Stmt) -> Option<SrcPos> {
    use pycc_ast::Stmt;
    let pos = stmt_location(stmt)?;
    let own: Vec<&pycc_ast::Expr> = match stmt {
        Stmt::Expr(s) => vec![&s.value],
        Stmt::Return(s) => s.value.iter().map(AsRef::as_ref).collect(),
        Stmt::Assign(s) => std::iter::once(s.value.as_ref())
            .chain(&s.targets)
            .collect(),
        Stmt::AugAssign(s) => vec![&s.target, &s.value],
        Stmt::AnnAssign(s) => s.value.iter().map(AsRef::as_ref).collect(),
        Stmt::If(s) => vec![&s.test],
        Stmt::While(s) => vec![&s.test],
        Stmt::For(s) => vec![&s.iter],
        Stmt::With(s) => s.items.iter().map(|i| &i.context_expr).collect(),
        Stmt::Match(s) => vec![&s.subject],
        Stmt::Raise(s) => s.exc.iter().chain(&s.cause).map(AsRef::as_ref).collect(),
        Stmt::Assert(s) => std::iter::once(s.test.as_ref())
            .chain(s.msg.as_deref())
            .collect(),
        Stmt::Delete(s) => s.targets.iter().collect(),
        _ => Vec::new(),
    };
    Some(SrcPos {
        sites: register_sites(&own),
        ..pos
    })
}

/// The location of `stmt`'s marker (module documentation).
fn stmt_location(stmt: &pycc_ast::Stmt) -> Option<SrcPos> {
    use pycc_ast::{Expr, Ranged, Stmt, TextRange};
    let range: TextRange = match stmt {
        Stmt::Expr(s) => s.value.range(),
        Stmt::Return(s) => s.value.as_ref().map_or(s.range, |v| v.range()),
        // A store whose value cannot fail fails at its target.
        Stmt::Assign(s)
            if cannot_fail(&s.value)
                && s.targets.len() == 1
                && matches!(s.targets[0], Expr::Attribute(_) | Expr::Subscript(_)) =>
        {
            s.targets[0].range()
        }
        Stmt::Assign(s) => s.value.range(),
        Stmt::AnnAssign(s) => s.value.as_ref().map_or(s.range, |v| v.range()),
        Stmt::If(s) => s.test.range(),
        Stmt::While(s) => s.test.range(),
        Stmt::For(s) => s.iter.range(),
        Stmt::With(s) => s.items.first().map_or(s.range, |i| i.context_expr.range()),
        Stmt::Match(s) => s.subject.range(),
        // A compound or definition statement: only its first line.
        Stmt::FunctionDef(_) | Stmt::ClassDef(_) | Stmt::Try(_) => {
            return header_position(stmt.range().start().into());
        }
        Stmt::Pass(_) | Stmt::Break(_) | Stmt::Continue(_) => return None,
        other => other.range(),
    };
    position_of(range.start().into(), range.end().into())
}

/// The position of a compound statement's first line, with no columns:
/// a function's header marker (`co_firstlineno`), a class or `try` line.
fn header_position(start: u32) -> Option<SrcPos> {
    position_of(start, start + 1).map(|p| SrcPos {
        end_line: p.line,
        col: u32::MAX,
        end_col: u32::MAX,
        ..p
    })
}

/// A function body's header marker: the position of its `def` (its first
/// decorator, as `co_firstlineno` counts), put first in the lowered body.
pub(crate) fn def_header(def: &pycc_ast::StmtFunctionDef) -> Option<crate::HirStmt> {
    header_position(def.range.start().into()).map(crate::HirStmt::Pos)
}
