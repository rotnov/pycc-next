//! The fallback cascade: statements whose compiled form fallback code
//! could not see fall back too, and the module globals both sides share
//! through the namespace dictionary.

use super::*;

impl Plan {
    /// Adds the statements whose compiled form Python code could not see:
    /// a compiled import, function or class that fallback code reads
    /// (unless the module publishes the function or class).
    pub(super) fn cascade(&mut self) {
        loop {
            let globals = self.dict_globals();
            let read: BTreeSet<&String> = self
                .fallback
                .keys()
                .chain(&self.mirrored)
                .flat_map(|index| &self.stmts[*index].mentions)
                .chain(&self.external)
                .collect();
            // A name fallback code reads that no top-level statement binds
            // may come from a `from m import *`, whose names are not known
            // here: such an import falls back with the first one.
            let all_bound: BTreeSet<&String> =
                self.stmts.iter().flat_map(|stmt| &stmt.bound).collect();
            let unbound_read = read.iter().find(|name| !all_bound.contains(*name)).copied();
            let mirror: Vec<usize> = (0..self.stmts.len())
                .filter(|index| {
                    !self.fallback.contains_key(index)
                        && !self.mirrored.contains(index)
                        && matches!(self.stmts[*index].kind, Kind::Import | Kind::Other)
                        && self.stmts[*index].bound.iter().any(|n| read.contains(n))
                        && self.mirrorable(*index)
                })
                .collect();
            if !mirror.is_empty() {
                self.mirrored.extend(mirror);
                continue;
            }
            let objectify: Vec<usize> = (0..self.stmts.len())
                .filter(|index| {
                    !self.fallback.contains_key(index)
                        && !self.objectified.contains(index)
                        && matches!(&self.stmts[*index].kind,
                            Kind::Def(name, true) if read.contains(name)
                                && !self.stmts[*index].defaults
                                && self.publishes.sees(name, false, false)
                                && self.publishes.infers_unannotated)
                })
                .collect();
            if !objectify.is_empty() {
                self.objectified.extend(objectify);
                continue;
            }
            // Issue #1: a compiled `def` whose name fallback code binds
            // too (a redefinition in a block, `f = lambda ...`, a function
            // declaring it `global`) runs in CPython as well, so the name
            // has one home, the namespace, which compiled callers read.
            let fallback_binds: BTreeSet<&String> = self
                .fallback
                .keys()
                .flat_map(|index| {
                    let stmt = &self.stmts[*index];
                    stmt.bound.iter().chain(&stmt.declares)
                })
                .collect();
            // So does a function defined more than once that fallback code
            // reads: the module publishes a redefined function only once
            // its last `def` has run (`pycc_codegen::ext_publish`), so
            // Python code reading an earlier one would find no name.
            let mut compiled_defs: BTreeMap<&String, usize> = BTreeMap::new();
            for (index, stmt) in self.stmts.iter().enumerate() {
                if let Kind::Def(name, _) = &stmt.kind
                    && !self.fallback.contains_key(&index)
                {
                    *compiled_defs.entry(name).or_default() += 1;
                }
            }
            let rebound: Vec<(usize, String)> = (0..self.stmts.len())
                .filter(|index| !self.fallback.contains_key(index))
                .filter_map(|index| match &self.stmts[index].kind {
                    Kind::Def(name, _) if fallback_binds.contains(name) => Some((
                        index,
                        format!(
                            "fallback code rebinds `{name}`, which compiled code calls natively"
                        ),
                    )),
                    Kind::Def(name, _) if read.contains(name) && compiled_defs[name] > 1 => Some((
                        index,
                        format!(
                            "fallback code uses `{name}`, which Python code cannot see compiled \
                             before its last definition"
                        ),
                    )),
                    _ => None,
                })
                .collect();
            if !rebound.is_empty() {
                self.fallback.extend(rebound);
                continue;
            }
            // A compiled class fallback code subclasses: CPython accepts no
            // published class as a base.
            let subclassed: BTreeSet<&String> = self
                .fallback
                .keys()
                .flat_map(|index| &self.stmts[*index].bases)
                .collect();
            let forced: Vec<(usize, String)> = self
                .stmts
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    !self.fallback.contains_key(index) && !self.mirrored.contains(index)
                })
                .filter_map(|(index, stmt)| {
                    let name = match &stmt.kind {
                        Kind::Import if stmt.opaque.contains("*") => unbound_read?,
                        Kind::Import | Kind::Other => {
                            stmt.opaque.iter().find(|n| globals.contains(*n))?
                        }
                        Kind::Def(name, unannotated)
                            if read.contains(name)
                                && (stmt.defaults || !self.publishes.sees(
                                    name,
                                    false,
                                    *unannotated && !self.objectified.contains(&index),
                                )) =>
                        {
                            name
                        }
                        Kind::Class(name)
                            if read.contains(name) && !self.publishes.sees(name, true, false) =>
                        {
                            name
                        }
                        Kind::Class(name) if subclassed.contains(name) => {
                            return Some((
                                index,
                                format!(
                                    "fallback code subclasses `{name}`, which Python code cannot \
                                     subclass compiled"
                                ),
                            ));
                        }
                        Kind::Class(name) if self.package_bases.contains_key(name) => {
                            return Some((
                                index,
                                format!(
                                    "`{}` of the package subclasses `{name}`, which Python code \
                                     cannot subclass compiled",
                                    self.package_bases[name]
                                ),
                            ));
                        }
                        // Python code importing the module reads a public
                        // class too: one published without its members
                        // runs in CPython for it.
                        Kind::Class(name)
                            if stmt.hidden_members
                                && (read.contains(name)
                                    || self.publishes.variables && !name.starts_with('_')) =>
                        {
                            name
                        }
                        // An instance of a compiled class may reach fallback
                        // code through any value without its name being
                        // read, and an unpublished class's instances have no
                        // attributes Python code can see.
                        Kind::Class(name)
                            if !self.publishes.classes && !self.fallback.is_empty() =>
                        {
                            name
                        }
                        Kind::Def(..) | Kind::Class(_) => return None,
                    };
                    Some((
                        index,
                        format!(
                            "fallback code uses `{name}`, which Python code cannot see compiled"
                        ),
                    ))
                })
                .collect();
            if forced.is_empty() {
                return;
            }
            self.fallback.extend(forced);
        }
    }

    /// Demotes the native global whose namespace copy `failure` points at
    /// (a list cannot be stored into the namespace without a copy that
    /// would break aliasing), returning whether it did.
    pub(super) fn demote(&mut self, rendered: Option<&Rendered>, failure: &Failure) -> bool {
        let (Some(rendered), Some(span)) = (rendered, failure.diagnostic.span) else {
            return false;
        };
        let Some((_, name)) = rendered
            .mirror_spans
            .iter()
            .find(|(range, _)| range.contains(&span.start))
        else {
            return false;
        };
        self.demoted.insert(name.clone())
    }

    /// The dict globals (`dict_globals`) that compiled code keeps native
    /// instead, copying every compiled store into the namespace: fallback
    /// code reads them but never binds them, so compiled code is their one
    /// writer, and every compiled store is a plain `=`/`+=` a copy can
    /// follow. The copy only compiles for a value the namespace can hold
    /// without breaking aliasing (a scalar or an object); any other is
    /// demoted by the fallback loop.
    pub(super) fn native_globals(&self) -> BTreeSet<String> {
        let mut fallback_binds = BTreeSet::new();
        for (index, stmt) in self.stmts.iter().enumerate() {
            if self.fallback.contains_key(&index) {
                fallback_binds.extend(stmt.bound.iter().cloned());
                fallback_binds.extend(stmt.declares.iter().cloned());
            } else if !matches!(stmt.kind, Kind::Other) {
                fallback_binds.extend(stmt.opaque.iter().cloned());
                if index >= self.top && self.live(index) {
                    fallback_binds.extend(stmt.declares.iter().cloned());
                }
            }
        }
        let mut native: BTreeSet<String> = self
            .dict_globals()
            .into_iter()
            .filter(|name| !fallback_binds.contains(name) && !self.demoted.contains(name))
            .collect();
        let none = BTreeSet::new();
        for (index, stmt) in self.ast.iter().enumerate() {
            if self.fallback.contains_key(&index) || !self.live(index) {
                continue;
            }
            let mut rewriter = names::Rewriter::new(&none, NS).with_mirrors(native.clone());
            pycc_ast::visitor::Visitor::visit_stmt(&mut rewriter, stmt);
            native.retain(|name| !rewriter.unmirrorable.contains(name));
        }
        native
    }

    /// The module globals that live in the namespace dictionary.
    pub(super) fn dict_globals(&self) -> BTreeSet<String> {
        let mut fallback_bound = BTreeSet::new();
        let mut fallback_read = self.external.clone();
        let mut compiled_bound = BTreeSet::new();
        for (index, stmt) in self.stmts.iter().enumerate() {
            if self.fallback.contains_key(&index) {
                fallback_bound.extend(stmt.bound.iter().cloned());
                fallback_read.extend(stmt.mentions.iter().cloned());
            } else if self.twinned.contains_key(&index) {
                fallback_bound.extend(stmt.declares.iter().cloned());
                fallback_read.extend(stmt.mentions.iter().cloned());
            } else if index >= self.top && self.live(index) {
                // A compiled lift is called by CPython (its forwarder), so
                // a global it declares lives in the namespace, as one a
                // method running in CPython would bind (`lift`).
                fallback_bound.extend(stmt.declares.iter().cloned());
            } else if matches!(stmt.kind, Kind::Other | Kind::Import)
                && !self.mirrored.contains(&index)
            {
                compiled_bound.extend(stmt.bound.iter().cloned());
            }
        }
        let mut globals: BTreeSet<String> = fallback_read
            .intersection(&compiled_bound)
            .cloned()
            .collect();
        globals.extend(fallback_bound);
        // A compiled `def`/`class` name stays native: it is published, or
        // fallback code reading it made it fall back too (`cascade`).
        for (index, stmt) in self.stmts.iter().enumerate() {
            if let (false, Kind::Def(name, _) | Kind::Class(name)) =
                (self.fallback.contains_key(&index), &stmt.kind)
                && !compiled_bound.contains(name)
            {
                globals.remove(name);
            }
        }
        globals
    }

    /// The statement the preamble goes before: the first one that is not
    /// the module docstring or a `from __future__` import.
    pub(super) fn preamble_position(&self) -> usize {
        let body = &self.ast;
        let mut index = 0;
        if matches!(body.first(), Some(Stmt::Expr(e)) if e.value.is_string_literal_expr()) {
            index = 1;
        }
        while matches!(body.get(index), Some(Stmt::ImportFrom(import))
            if import.module.as_ref().map(|m| m.as_str()) == Some("__future__"))
        {
            index += 1;
        }
        index
    }
}
