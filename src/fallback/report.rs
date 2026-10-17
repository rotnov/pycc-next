//! `--report-fallbacks`: one line per piece of the entry module that runs
//! in CPython (#12), and per compiled function twinned there.
//!
//! A split statement (`lift`) is reported by what of it really runs in
//! CPython: its header (`class C(Base):`, decorators, class attributes),
//! each lifted method whose body falls back, and each `def` it holds that
//! is not lifted. Every line names the lines it covers, so a count of the
//! `def`s in them is the count of functions not compiled.

use super::*;

impl Plan {
    /// One line per fallback statement, for `--report-fallbacks`.
    pub(crate) fn report_lines(&self) -> Vec<String> {
        let twins = self.twinned.iter().map(|(index, reason)| {
            let stmt = &self.stmts[*index];
            let (first, last) = self.lines(stmt.range.clone());
            let name = match &stmt.kind {
                Kind::Def(name, _) => name.as_str(),
                _ => "?",
            };
            format!(
                "twin: function `{name}` at {}:{first}-{last} is compiled, and runs in CPython \
                 when Python code calls it: {reason}",
                self.display
            )
        });
        let mut lines = Vec::new();
        for (index, reason) in &self.fallback {
            if let Some(lift) = self.lift(*index) {
                // A module-level function is its own lift's statement (the
                // whole `def` runs in CPython); a `def` in a block is a
                // function too.
                let what = if matches!(self.stmts[lift.owner].kind, Kind::Class(_)) {
                    "method"
                } else {
                    "function"
                };
                let reason = if self.is_attached(*index) {
                    format!("{reason}; its class is compiled, the method attached to it")
                } else {
                    reason.clone()
                };
                lines.push(self.line(what, &lift.qualname, lift.range.clone(), &reason));
                continue;
            }
            if self.is_split(*index) && matches!(self.stmts[*index].kind, Kind::Def(..)) {
                // Reported by its lift when that falls back too.
                if self.lifts_of(*index).all(|unit| !self.fallback.contains_key(&unit)) {
                    let (first, last) = self.lines(self.stmts[*index].range.clone());
                    lines.push(format!(
                        "lifted: function `{}` at {}:{}: its `def` (decorators, defaults, \
                         signature) runs in CPython, its body is compiled: {reason}",
                        match &self.stmts[*index].kind {
                            Kind::Def(name, _) => name.as_str(),
                            _ => "?",
                        },
                        self.display,
                        span(first, last)
                    ));
                }
                continue;
            }
            if self.is_split(*index) {
                lines.extend(self.split_lines(*index, reason));
                continue;
            }
            let stmt = &self.stmts[*index];
            let (first, last) = self.lines(stmt.range.clone());
            let what = match &stmt.kind {
                Kind::Def(name, _) => format!("function `{name}`"),
                Kind::Class(name) => format!("class `{name}`"),
                Kind::Import | Kind::Other if first == last => "statement".to_string(),
                Kind::Import | Kind::Other => "statements".to_string(),
            };
            lines.push(format!(
                "fallback: {what} at {}:{} runs in CPython: {reason}",
                self.display,
                span(first, last)
            ));
        }
        lines.extend(twins);
        lines
    }

    /// The report of split statement `index`: its header, and each `def` in
    /// it that is not lifted (each lift falling back is reported as itself).
    fn split_lines(&self, index: usize, reason: &str) -> Vec<String> {
        let stmt = &self.stmts[index];
        let mut defs = Vec::new();
        collect_defs(std::slice::from_ref(&self.ast[index]), &mut defs);
        let lifted: BTreeSet<u32> = self
            .lifts_of(index)
            .filter_map(|unit| self.lift(unit))
            .map(|lift| lift.range.start)
            .collect();
        // A decorated function's header is its decorators.
        let header_end = match &stmt.kind {
            Kind::Def(..) => self
                .lifts_of(index)
                .filter_map(|unit| self.lift(unit))
                .map(|lift| lift.keyword)
                .next()
                .unwrap_or(stmt.range.end),
            _ => defs
                .iter()
                .map(|(range, _)| range.start)
                .min()
                .unwrap_or(stmt.range.end),
        };
        let (first, _) = self.lines(stmt.range.clone());
        let before = line_of(&self.source, header_end).saturating_sub(1).max(first);
        let what = match &stmt.kind {
            Kind::Def(name, _) => format!("decorators of function `{name}`"),
            Kind::Class(name) => format!("class `{name}`"),
            Kind::Import | Kind::Other => "statement".to_string(),
        };
        let compiled = if matches!(stmt.kind, Kind::Class(_)) {
            "methods"
        } else {
            "functions"
        };
        let mut out = vec![format!(
            "fallback: {what} at {}:{} runs in CPython, its {compiled} compiled: {reason}",
            self.display,
            span(first, before)
        )];
        for (range, name) in defs {
            if !lifted.contains(&range.start) {
                let what = if matches!(stmt.kind, Kind::Class(_)) { "method" } else { "function" };
                out.push(self.line(what, &name, range, "not lifted out of its statement"));
            }
        }
        out
    }

    fn line(&self, what: &str, name: &str, range: Range<u32>, reason: &str) -> String {
        let (first, last) = self.lines(range);
        format!(
            "fallback: {what} `{name}` at {}:{} runs in CPython: {reason}",
            self.display,
            span(first, last)
        )
    }

    fn lines(&self, range: Range<u32>) -> (usize, usize) {
        (
            line_of(&self.source, range.start),
            line_of(&self.source, range.end.saturating_sub(1)),
        )
    }
}

fn span(first: usize, last: usize) -> String {
    if first == last {
        format!("{first}")
    } else {
        format!("{first}-{last}")
    }
}

/// Every `def` in `body` outside a function (in a class body or a block),
/// with its range and qualified name.
fn collect_defs(body: &[Stmt], out: &mut Vec<(Range<u32>, String)>) {
    fn walk(body: &[Stmt], qual: &str, out: &mut Vec<(Range<u32>, String)>) {
        for stmt in body {
            match stmt {
                Stmt::FunctionDef(def) => {
                    out.push((def.range.into(), format!("{qual}{}", def.name.as_str())));
                }
                Stmt::ClassDef(class) => {
                    walk(&class.body, &format!("{qual}{}.", class.name.as_str()), out);
                }
                Stmt::If(s) => {
                    walk(&s.body, qual, out);
                    for clause in &s.elif_else_clauses {
                        walk(&clause.body, qual, out);
                    }
                }
                Stmt::For(s) => {
                    walk(&s.body, qual, out);
                    walk(&s.orelse, qual, out);
                }
                Stmt::While(s) => {
                    walk(&s.body, qual, out);
                    walk(&s.orelse, qual, out);
                }
                Stmt::With(s) => walk(&s.body, qual, out),
                Stmt::Try(s) => {
                    walk(&s.body, qual, out);
                    for handler in &s.handlers {
                        let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
                        walk(&h.body, qual, out);
                    }
                    walk(&s.orelse, qual, out);
                    walk(&s.finalbody, qual, out);
                }
                Stmt::Match(s) => {
                    for case in &s.cases {
                        walk(&case.body, qual, out);
                    }
                }
                _ => {}
            }
        }
    }
    walk(body, "", out);
}
