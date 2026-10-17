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
    /// Each method attached to a compiled class (`attach`): the class, the
    /// class statement's offset in `text`, and the attributes of `self`
    /// the method uses, which the frontend checks the carrier exposes.
    pub(super) attached: Vec<(String, u32, BTreeSet<String>)>,
    /// The CPython source `text` runs, in an `--ext` module: chunk `k` is
    /// what `__pycc_fallback_ns["__pycc_code__"](k)` returns the code
    /// object of (`CodeChunk`). Empty elsewhere, where `text` calls
    /// `compile()` itself.
    pub(super) chunks: Vec<CodeChunk>,
}

/// One piece of CPython source an `--ext` module runs with `exec`. The
/// build compiles it ahead of time with the target interpreter and embeds
/// the marshalled code object (`crate::ext_build::precompiled`), so an
/// import unmarshals it as CPython loads a `.pyc` instead of compiling the
/// source; the source is embedded too, compiled at run time when the
/// running interpreter's bytecode differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeChunk {
    /// The source, padded with newlines so its line numbers are the
    /// entry file's.
    pub(crate) source: String,
    /// The `co_filename` (the entry file's display path).
    pub(crate) filename: String,
    /// Compiled with `from __future__ import annotations`.
    pub(crate) future_annotations: bool,
}

/// The name, in an `--ext` module's namespace while its body runs, of the
/// function returning a fallback chunk's code object (`CodeChunk`).
const CODE_HOOK: &str = "__pycc_code__";

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

/// The rewrite being built.
struct Out {
    text: String,
    ranges: Vec<Range<u32>>,
    copied: Vec<(u32, u32, u32)>,
    mirror_spans: Vec<(Range<u32>, String)>,
    edited: Vec<(Range<u32>, u32)>,
    chunks: Vec<CodeChunk>,
}

/// The range of a unit with no text of its own in the rewrite (a lift whose
/// statement is compiled, or that falls back with it): no offset is in it.
const NOWHERE: Range<u32> = u32::MAX..u32::MAX;

impl Plan {
    /// The rewritten source.
    pub(crate) fn render(&self) -> Rendered {
        let native = self.native_globals();
        let globals: BTreeSet<String> = self.dict_globals().difference(&native).cloned().collect();
        let src = &self.source;
        let mut out = Out {
            text: String::with_capacity(src.len() * 2),
            ranges: vec![NOWHERE; self.stmts.len()],
            copied: Vec::new(),
            mirror_spans: Vec::new(),
            edited: Vec::new(),
            chunks: Vec::new(),
        };
        let mut trailing: Vec<String> = Vec::new();
        let mut cursor = 0usize;
        let preamble_at = self.preamble_position();
        let mut preamble = 0..0;
        let mut index = 0;
        // A copy to run first thing on the next statement's line (`copy; `).
        let mut prefix: Option<String> = None;
        while index < self.top {
            let stmt = &self.stmts[index];
            let start = stmt.range.start as usize;
            out.text.push_str(&src[cursor..start]);
            if let Some(run) = prefix.take() {
                out.text.push_str(&run);
                out.text.push_str("; ");
            }
            if index == preamble_at {
                let at = out.text.len() as u32;
                out.text.push_str(&format!(
                    "import sys as {SYS}; import builtins as {BUILTINS}; \
                     {NS} = {SYS}.modules[__name__].__dict__"
                ));
                // The methods attached to compiled classes (`attach`),
                // defined in CPython before anything can call them.
                for unit in self.attached() {
                    if let Some(source) = self.attached_source(unit) {
                        out.text.push_str("; ");
                        out.text.push_str(&self.exec_text(&mut out.chunks, &source));
                    }
                }
                // The nested statements' scaffolds (`nested`), defined
                // before any compiled function can run one.
                let nested = self.nested_preamble(&mut out.chunks);
                out.text.push_str(&nested);
                out.text.push('\n');
                preamble = at..out.text.len() as u32;
                // The compiled bodies of nested `def`s (`nested`), defined
                // before any compiled function can make one.
                for unit in self.nested_body_units() {
                    let at = out.text.len() as u32;
                    self.emit_compiled(&mut out, unit, &globals, &native);
                    out.ranges[unit] = at..out.text.len() as u32;
                    out.text.push('\n');
                }
            }
            if self.is_split(index) {
                cursor = self.render_split(index, &mut out, &globals, &native, &mut prefix);
                index += 1;
                continue;
            }
            if self.fallback.contains_key(&index) {
                let mut last = index;
                while last + 1 < self.top
                    && self.fallback.contains_key(&(last + 1))
                    && !self.is_split(last + 1)
                {
                    last += 1;
                }
                let end = self.stmts[last].range.end as usize;
                let chunk = &src[start..end];
                let line = src[..start].matches('\n').count();
                let padded = format!("{}{}\n", "\n".repeat(line), chunk);
                // The newlines go first, so the call sits on the chunk's last
                // line: a statement may follow it there after a `;`.
                out.text.push_str(&"\n".repeat(chunk.matches('\n').count()));
                let at = out.text.len() as u32;
                out.text.push_str(&self.exec_text(&mut out.chunks, &padded));
                let end_at = out.text.len() as u32;
                for unit in index..=last {
                    out.ranges[unit] = at..end_at;
                }
                cursor = end;
                index = last + 1;
                continue;
            }
            let at = out.text.len() as u32;
            self.emit_compiled(&mut out, index, &globals, &native);
            let end = stmt.range.end as usize;
            if self.mirrored.contains(&index) {
                // Padded, so a traceback through the CPython copy names
                // its lines.
                let line = src[..start].matches('\n').count();
                let run = self.exec_text(
                    &mut out.chunks,
                    &format!("{}{}", "\n".repeat(line), &src[start..end]),
                );
                if !matches!(self.stmts[index].kind, Kind::Def(..)) {
                    // Same line, so later line numbers do not move.
                    out.text.push_str("; ");
                    out.text.push_str(&run);
                } else if let Some(blank) = blank_line_after(src, end) {
                    // A `def`'s last line is in its body: the copy runs on
                    // the blank line after it, so no line number moves.
                    out.ranges[index] = at..out.text.len() as u32;
                    out.text.push_str(&src[end..blank]);
                    out.text.push_str(&run);
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
            out.ranges[index] = at..out.text.len() as u32;
            cursor = end;
            index += 1;
        }
        let mut text = out.text;
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
        let attached = self
            .attached()
            .into_iter()
            .filter_map(|unit| {
                let lift = self.lift(unit)?;
                let attach = lift.attach.as_ref()?;
                Some((
                    attach.class.clone(),
                    out.ranges[lift.owner].start,
                    attach.self_attrs.clone(),
                ))
            })
            .collect();
        Rendered {
            attached,
            chunks: out.chunks,
            text,
            ranges: out.ranges,
            preamble,
            copied: out.copied,
            mirror_spans: out.mirror_spans,
            edited: out.edited,
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

    /// `exec`s the (padded) CPython source `source` in the namespace: in
    /// an `--ext` module the code object of a new chunk of `chunks`
    /// (`CodeChunk`), elsewhere `compile()`d where it runs.
    pub(super) fn exec_text(&self, chunks: &mut Vec<CodeChunk>, source: &str) -> String {
        if self.publishes.variables {
            chunks.push(CodeChunk {
                source: source.to_string(),
                filename: self.display.clone(),
                future_annotations: self.defers_annotations,
            });
            return format!(
                "{BUILTINS}.exec({NS}[\"{CODE_HOOK}\"]({}), {NS})",
                chunks.len() - 1
            );
        }
        format!(
            "{BUILTINS}.exec({BUILTINS}.compile({}, {}, \"exec\"{}), {NS})",
            py_str(source),
            py_str(&self.display),
            self.compile_flags()
        )
    }

    /// Appends compiled unit `index` (a top-level statement, or a lift) to
    /// `out`: its source from `stmts[index].range`, with every edit.
    fn emit_compiled(
        &self,
        out: &mut Out,
        index: usize,
        globals: &BTreeSet<String>,
        native: &BTreeSet<String>,
    ) {
        let src = &self.source;
        let stmt = &self.stmts[index];
        let start = stmt.range.start as usize;
        let mut rewriter = names::Rewriter::new(globals, NS).with_mirrors(native.clone());
        pycc_ast::visitor::Visitor::visit_stmt(&mut rewriter, &self.ast[index]);
        let mut edits = rewriter.edits;
        let plain = |(at, (end, text)): (&u32, &(u32, String))| names::Edit {
            range: *at..*end,
            text: text.clone(),
            mirror: None,
        };
        let range = stmt.range.start..stmt.range.end;
        edits.extend(self.object_builtins.range(range.clone()).map(plain));
        edits.extend(
            self.erased
                .range(range.clone())
                .chain(self.substituted.range(range))
                .map(plain),
        );
        if self.objectified.contains(&index)
            && let Stmt::FunctionDef(def) = &self.ast[index]
        {
            edits.extend(object_annotations(def));
        }
        // The bodies of the methods attached to this compiled class
        // (`attach`) are forwarders to their CPython functions.
        if index < self.top {
            for unit in self.lifts_of(index).collect::<Vec<_>>() {
                if !self.is_attached(unit) {
                    continue;
                }
                if let Some((body, text)) = self.attached_forwarder(unit) {
                    edits.retain(|edit| !(body.start <= edit.range.start && edit.range.end <= body.end));
                    edits.push(names::Edit {
                        range: body,
                        text,
                        mirror: None,
                    });
                }
            }
        }
        edits.extend(self.nested_edits(index));
        if let Some(lift) = self.lift(index) {
            edits.extend(lift.edits.iter().map(|(range, text)| names::Edit {
                range: range.clone(),
                text: text.clone(),
                mirror: None,
            }));
        }
        // An insertion goes before a replacement starting where it does.
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        let mut inner = start;
        for edit in edits {
            let (edit_start, edit_end) = (edit.range.start as usize, edit.range.end as usize);
            if edit_start < inner {
                continue;
            }
            out.copied
                .push((out.text.len() as u32, inner as u32, (edit_start - inner) as u32));
            out.text.push_str(&src[inner..edit_start]);
            if let Some(name) = edit.mirror {
                let at = out.text.len() as u32;
                out.mirror_spans.push((at..at + edit.text.len() as u32, name));
            }
            let at = out.text.len() as u32;
            out.text.push_str(&edit.text);
            out.edited.push((at..out.text.len() as u32, edit_start as u32));
            inner = edit_end;
        }
        let end = stmt.range.end as usize;
        out.copied
            .push((out.text.len() as u32, inner as u32, (end - inner) as u32));
        out.text.push_str(&src[inner..end]);
    }

    /// Appends split statement `index` (`lift`): each compiled lift at its
    /// own lines, then the statement's `exec` once they are all defined.
    /// Returns the source offset rendering continues from.
    fn render_split(
        &self,
        index: usize,
        out: &mut Out,
        globals: &BTreeSet<String>,
        native: &BTreeSet<String>,
        prefix: &mut Option<String>,
    ) -> usize {
        let src = &self.source;
        let stmt = &self.stmts[index];
        let (start, end) = (stmt.range.start as usize, stmt.range.end as usize);
        let mut pos = start;
        let mut compiled = false;
        let units: Vec<usize> = self.lifts_of(index).collect();
        for unit in units {
            if self.fallback.contains_key(&unit) {
                continue;
            }
            let lift = self.lift(unit).expect("a lift");
            out.text
                .push_str(&"\n".repeat(src[pos..lift.keyword as usize].matches('\n').count()));
            let at = out.text.len() as u32;
            self.emit_compiled(out, unit, globals, native);
            out.ranges[unit] = at..out.text.len() as u32;
            pos = lift.range.end as usize;
            compiled = true;
        }
        let rest = src[pos..end].matches('\n').count();
        out.text.push_str(&"\n".repeat(rest));
        let line = src[..start].matches('\n').count();
        let run = self.exec_text(
            &mut out.chunks,
            &format!("{}{}\n", "\n".repeat(line), self.split_source(index)),
        );
        if !compiled || rest > 0 {
            // The statement's last line holds no compiled code.
            let at = out.text.len() as u32;
            out.text.push_str(&run);
            out.ranges[index] = at..out.text.len() as u32;
            return end;
        }
        if let Some(blank) = blank_line_after(src, end) {
            out.text.push_str(&src[end..blank]);
            let at = out.text.len() as u32;
            out.text.push_str(&run);
            out.ranges[index] = at..out.text.len() as u32;
            return blank;
        }
        if self.ast.get(index + 1).is_some_and(is_simple_statement) {
            *prefix = Some(run);
            return end;
        }
        // No free line after the statement: the `exec` takes one of its
        // own, and the lines after it move by one.
        out.text.push('\n');
        let at = out.text.len() as u32;
        out.text.push_str(&run);
        out.ranges[index] = at..out.text.len() as u32;
        end
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
