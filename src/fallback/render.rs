//! Rendering a fallback plan (#12) into the source the frontend compiles.

use super::*;

/// The rewrite of a plan, with where each original statement landed.
#[derive(Debug)]
pub(crate) struct Rendered {
    pub(crate) text: String,
    /// `ranges[i]` is statement `i`'s range in `text`.
    pub(super) ranges: Vec<Range<u32>>,
    pub(super) preamble: Range<u32>,
    /// `(offset in text, offset in the source, length)` of every run of
    /// compiled source copied into `text` unchanged.
    pub(super) copied: Vec<(u32, u32, u32)>,
    /// Where each copy of a native global into the namespace was inserted
    /// (`Plan::native_globals`), and the global's name.
    pub(super) mirror_spans: Vec<(Range<u32>, String)>,
    /// `(range in text, source offset)` of every edit's replacement text,
    /// with the offset of the source text it replaced.
    pub(super) edited: Vec<(Range<u32>, u32)>,
    /// The names of the twinned functions (`Plan::twinned`).
    pub(super) twinned: BTreeSet<String>,
}

impl Rendered {
    /// The source offset of `text` offset `at`, when it lies in copied
    /// compiled source.
    pub(super) fn source_offset(&self, at: u32) -> Option<u32> {
        self.copied
            .iter()
            .find(|(start, _, len)| (*start..start + len).contains(&at))
            .map(|(start, source, _)| source + (at - start))
    }

    /// The source offset of `text` offset `at`: where it was copied from,
    /// or the start of the source text an edit replaced there.
    pub(super) fn source_or_edit_offset(&self, at: u32) -> Option<u32> {
        self.source_offset(at).or_else(|| {
            self.edited
                .iter()
                .find(|(range, _)| range.contains(&at))
                .map(|(_, source)| *source)
        })
    }
}

impl Plan {
    /// The rewritten source.
    pub(crate) fn render(&self) -> Rendered {
        let native = self.native_globals();
        let globals: BTreeSet<String> = self.dict_globals().difference(&native).cloned().collect();
        let mut mirror_spans = Vec::new();
        let src = &self.source;
        let mut text = String::with_capacity(src.len() * 2);
        let mut ranges = Vec::with_capacity(self.stmts.len());
        let mut copied = Vec::new();
        let mut edited = Vec::new();
        let mut trailing: Vec<String> = Vec::new();
        let mut cursor = 0usize;
        let preamble_at = self.preamble_position();
        let mut preamble = 0..0;
        let mut index = 0;
        // A copy to run first thing on the next statement's line (`copy; `).
        let mut prefix: Option<String> = None;
        while index < self.stmts.len() {
            let stmt = &self.stmts[index];
            let start = stmt.range.start as usize;
            text.push_str(&src[cursor..start]);
            if let Some(run) = prefix.take() {
                text.push_str(&run);
                text.push_str("; ");
            }
            if index == preamble_at {
                let at = text.len() as u32;
                text.push_str(&format!(
                    "import sys as {SYS}; import builtins as {BUILTINS}; \
                     {NS} = {SYS}.modules[__name__].__dict__\n"
                ));
                preamble = at..text.len() as u32;
            }
            if self.fallback.contains_key(&index) {
                let mut last = index;
                while last + 1 < self.stmts.len() && self.fallback.contains_key(&(last + 1)) {
                    last += 1;
                }
                let end = self.stmts[last].range.end as usize;
                let chunk = &src[start..end];
                let line = src[..start].matches('\n').count();
                let padded = format!("{}{}\n", "\n".repeat(line), chunk);
                let at = text.len() as u32;
                // The newlines go first, so the call sits on the chunk's last
                // line: a statement may follow it there after a `;`.
                text.push_str(&"\n".repeat(chunk.matches('\n').count()));
                text.push_str(&format!(
                    "{BUILTINS}.exec({BUILTINS}.compile({}, {}, \"exec\"{}), {NS})",
                    py_str(&padded),
                    py_str(&self.display),
                    self.compile_flags()
                ));
                let end_at = text.len() as u32;
                for _ in index..=last {
                    ranges.push(at..end_at);
                }
                cursor = end;
                index = last + 1;
                continue;
            }
            let mut rewriter = names::Rewriter::new(&globals, NS).with_mirrors(native.clone());
            pycc_ast::visitor::Visitor::visit_stmt(&mut rewriter, &self.ast[index]);
            let mut edits = rewriter.edits;
            edits.extend(
                self.object_builtins
                    .range(stmt.range.start..stmt.range.end)
                    .map(|(at, (end, text))| names::Edit {
                        range: *at..*end,
                        text: text.clone(),
                        mirror: None,
                    }),
            );
            edits.extend(
                self.erased
                    .range(stmt.range.start..stmt.range.end)
                    .chain(self.substituted.range(stmt.range.start..stmt.range.end))
                    .map(|(at, (end, text))| names::Edit {
                        range: *at..*end,
                        text: text.clone(),
                        mirror: None,
                    }),
            );
            if self.objectified.contains(&index)
                && let Stmt::FunctionDef(def) = &self.ast[index]
            {
                edits.extend(object_annotations(def));
            }
            // An insertion goes before a replacement starting where it does.
            edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
            let at = text.len() as u32;
            let mut inner = start;
            for edit in edits {
                let (edit_start, edit_end) = (edit.range.start as usize, edit.range.end as usize);
                if edit_start < inner {
                    continue;
                }
                copied.push((text.len() as u32, inner as u32, (edit_start - inner) as u32));
                text.push_str(&src[inner..edit_start]);
                if let Some(name) = edit.mirror {
                    let at = text.len() as u32;
                    mirror_spans.push((at..at + edit.text.len() as u32, name));
                }
                let at = text.len() as u32;
                text.push_str(&edit.text);
                edited.push((at..text.len() as u32, edit_start as u32));
                inner = edit_end;
            }
            let end = stmt.range.end as usize;
            copied.push((text.len() as u32, inner as u32, (end - inner) as u32));
            text.push_str(&src[inner..end]);
            if self.mirrored.contains(&index) {
                // Padded, so a traceback through the CPython copy names
                // its lines.
                let line = src[..start].matches('\n').count();
                let run = format!(
                    "{BUILTINS}.exec({BUILTINS}.compile({}, {}, \"exec\"{}), {NS})",
                    py_str(&format!("{}{}", "\n".repeat(line), &src[start..end])),
                    py_str(&self.display),
                    self.compile_flags()
                );
                if !matches!(self.stmts[index].kind, Kind::Def(..)) {
                    // Same line, so later line numbers do not move.
                    text.push_str("; ");
                    text.push_str(&run);
                } else if let Some(blank) = blank_line_after(src, end) {
                    // A `def`'s last line is in its body: the copy runs on
                    // the blank line after it, so no line number moves.
                    ranges.push(at..text.len() as u32);
                    text.push_str(&src[end..blank]);
                    text.push_str(&run);
                    cursor = blank;
                    index += 1;
                    continue;
                } else if self.ast.get(index + 1).is_some_and(is_simple_statement) {
                    // No blank line after the `def`: the copy runs at the
                    // start of the next (simple) statement's line, before
                    // anything there can call it, and no line number moves.
                    prefix = Some(run);
                } else {
                    trailing.push(run);
                }
            }
            ranges.push(at..text.len() as u32);
            cursor = end;
            index += 1;
        }
        text.push_str(&src[cursor..]);
        // The copies with no blank line after their `def`, after the last
        // line, where no line number moves.
        for run in trailing {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&run);
            text.push('\n');
        }
        Rendered {
            text,
            ranges,
            preamble,
            copied,
            mirror_spans,
            edited,
            twinned: self
                .twinned
                .keys()
                .filter_map(|index| match &self.stmts[*index].kind {
                    Kind::Def(name, _) => Some(name.clone()),
                    _ => None,
                })
                .collect(),
        }
    }
}

/// Whether `stmt` is a simple statement, which a `copy; ` prefix on its
/// line can precede.
fn is_simple_statement(stmt: &Stmt) -> bool {
    !matches!(
        stmt,
        Stmt::FunctionDef(_)
            | Stmt::ClassDef(_)
            | Stmt::If(_)
            | Stmt::For(_)
            | Stmt::While(_)
            | Stmt::With(_)
            | Stmt::Try(_)
            | Stmt::Match(_)
    )
}

/// The offset of the first line after the line `end` is on, when that
/// line is blank (whitespace only), in `src`.
fn blank_line_after(src: &str, end: usize) -> Option<usize> {
    let line_end = end + src[end..].find('\n')?;
    let next = line_end + 1;
    let next_end = next + src[next..].find('\n')?;
    src[next..next_end].trim().is_empty().then_some(next)
}

impl Plan {
    /// The extra `compile()` arguments of fallback code: the module's
    /// `from __future__ import annotations`, which `exec`'d source does not
    /// inherit, so its annotations stay unevaluated there too.
    fn compile_flags(&self) -> String {
        if self.defers_annotations {
            format!(", {BUILTINS}.__import__(\"__future__\").annotations.compiler_flag")
        } else {
            String::new()
        }
    }
}

/// The edits annotating every unannotated parameter and the return of
/// `def` as `object` (`Plan::objectified`).
fn object_annotations(def: &pycc_ast::StmtFunctionDef) -> Vec<names::Edit> {
    let insert = |at: u32, text: &str| names::Edit {
        range: at..at,
        text: text.to_string(),
        mirror: None,
    };
    let mut edits: Vec<names::Edit> = def
        .parameters
        .iter()
        .filter(|p| p.annotation().is_none())
        .map(|p| insert(p.name().range.end().to_u32(), ": object"))
        .collect();
    if def.returns.is_none() {
        edits.push(insert(def.parameters.range.end().to_u32(), " -> object"));
    }
    edits
}
