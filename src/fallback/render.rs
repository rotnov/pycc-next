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
        let mut cursor = 0usize;
        let preamble_at = self.preamble_position();
        let mut preamble = 0..0;
        let mut index = 0;
        while index < self.stmts.len() {
            let stmt = &self.stmts[index];
            let start = stmt.range.start as usize;
            text.push_str(&src[cursor..start]);
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
                    "{BUILTINS}.exec({BUILTINS}.compile({}, {}, \"exec\"), {NS})",
                    py_str(&padded),
                    py_str(&self.display)
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
                text.push_str(&edit.text);
                inner = edit_end;
            }
            let end = stmt.range.end as usize;
            copied.push((text.len() as u32, inner as u32, (end - inner) as u32));
            text.push_str(&src[inner..end]);
            if self.mirrored.contains(&index) {
                // Same line, so later line numbers do not move.
                text.push_str(&format!(
                    "; {BUILTINS}.exec({BUILTINS}.compile({}, {}, \"exec\"), {NS})",
                    py_str(&src[start..end]),
                    py_str(&self.display)
                ));
            }
            ranges.push(at..text.len() as u32);
            cursor = end;
            index += 1;
        }
        text.push_str(&src[cursor..]);
        Rendered {
            text,
            ranges,
            preamble,
            copied,
            mirror_spans,
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
