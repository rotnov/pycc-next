//! [`LoweredModule`], one module's lowering result before
//! `program::link`/`program::finalize`.
//!
//! Extracted from `module.rs` per AGENTS.md's file-decomposition rule
//! (#1425), which added the `object_receivers` field: a pure move of the
//! struct and its documentation, re-exported from `module` so every
//! `crate::LoweredModule` path is unchanged, and of the `strip_imported`
//! helper that prepares its class and alias lists.

use crate::HirModule;
use crate::class::slots::ClassSlotsRow;
use pycc_diag::Span;
use std::collections::BTreeSet;

/// One module's lowering, before `program::link`/`program::finalize`
/// (#898). `shadowed_builtin_exception_name` is the first builtin exception
/// name this module's top level binds, if any -- the input to `link`'s
/// cross-module seeding check, since a module that shadows a builtin
/// exception name is never seeded itself but cannot be linked with a module that
/// was. `definition_spans` feeds `link`'s collision diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct LoweredModule {
    pub hir: HirModule,
    pub shadowed_builtin_exception_name: Option<String>,
    pub definition_spans: Vec<(String, Span)>,
    /// Whether this module mentions `__name__` at all -- a read as much as a
    /// binding, at any depth (W0 of #882, #1156). Published so the driver
    /// (`src/modules.rs`) can apply it to every *dependency*: Part 1 of #881
    /// links the program into one flat namespace and places every dependency's
    /// top-level statements ahead of the entry module's seed, so a dependency
    /// that touches the name at all either collides with the seed or reads the
    /// global before the seed stores anything. Withholding the seed
    /// program-wide is the fail-closed answer to both.
    ///
    /// Deliberately stricter than the entry module's own gate inside
    /// `dunder_name::seed_item`, which is a binding test: a dependency's read
    /// is the case a binding test cannot see, and it need not be textually
    /// top-level, because a top-level call to one of the dependency's own
    /// functions reaches a function-body read the same way. Publishing the
    /// predicate rather than re-deriving it keeps one answer to one question --
    /// an earlier revision inferred it from `definition_spans` instead, which
    /// records neither import bindings nor anything nested inside a top-level
    /// compound statement, and so answered "no" for both.
    pub mentions_dunder_name: bool,
    /// Issue #1188: which of `append`, `pop`, `get` and `add` a class
    /// reachable from this module defines as a method -- its own top-level
    /// classes plus everything its dependencies reach. The driver hands it to
    /// every module that imports this one, so the set follows the import
    /// closure rather than the set of modules loaded so far.
    pub container_method_names: BTreeSet<&'static str>,
    /// #1244: every name a module-scope `del` deletes, with the span of its
    /// `del` statement. `program::link` refuses a program in which another
    /// module mentions one of these names: the linked program is one flat
    /// namespace, so that module's read would see the deleted global.
    pub deleted_top_level: Vec<(String, Span)>,
    /// #1244: every name this module mentions anywhere (each `Expr::Name`
    /// id, a read or a store), for the same `program::link` rule. `None` from
    /// `lower_module`: the walk costs every module on every build, and only a
    /// multi-module program in which some module deletes a top-level name
    /// needs it, so the driver fills it from [`crate::mentioned_names`] in
    /// exactly that case, before calling `program::link`.
    pub mentioned_names: Option<BTreeSet<String>>,
    /// #1368: one row per class this module authors -- its name and, when
    /// its body binds `__slots__`, its own slot list (`class::slots`). The
    /// driver hands it to every importing module alongside `hir`, so a
    /// subclass there checks its layout and stores against the base's
    /// slots.
    pub class_slots: Vec<ClassSlotsRow>,
    /// Issue #1425: whether this module can hold a CPython object -- its
    /// *final* admission state, read after the whole module is lowered. It
    /// counts the module's own top-level and block foreign imports (decided
    /// before the first statement is lowered since #1482, see
    /// `module::own_foreign`) as well as what it inherited
    /// (`ResolvedImports::inherit_object_receivers`).
    /// The driver hands it to every module that imports this one, which then
    /// lowers its container-method calls with both readings for its whole
    /// body. The `SignatureTable` itself stays lowering-internal; only this
    /// bit is published.
    pub object_receivers: bool,
}

/// Drops the entries at `imported_indices` (a project import's copied
/// classes or aliases) from `entries`, keeping every other entry in order.
pub(super) fn strip_imported<T>(entries: Vec<T>, imported_indices: &[usize]) -> Vec<T> {
    entries
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !imported_indices.contains(index))
        .map(|(_, entry)| entry)
        .collect()
}
