//! The per-module HIR container (`HirModule`), its compile-time-only
//! import side table (`ImportBinding`), and the `killed_names` walk the type
//! checker's narrowing pass runs over a module's statements.
//!
//! Extracted from `lib.rs` per AGENTS.md's file-decomposition rule (issue
//! #898): these three definitions are the module-level surface the driver
//! and `pycc_types` consume, as opposed to the statement/expression node
//! types that stay in `lib.rs`.

use crate::{FStringPart, HirClassDef, HirExpr, HirItem, HirPattern, HirStmt, Ty};
use pycc_diag::Span;
use std::collections::HashSet;

/// Issue #769 follow-up (D-068 re-review of #780, third round): the set of
/// bare names `body` reassigns anywhere within it, recursively -- every
/// name that some execution of `body` could pass to
/// `pycc_types::check_assignment` (checker) or its MIR lowering
/// counterpart, which is exactly the set of names whose `Optional`
/// narrowing overlay entry that assignment kills.
///
/// This exists to support a *kill-prescan*: before checking/lowering a
/// body that can be entered such that a read inside it observes a kill
/// from an execution earlier than that read's own source position -- a
/// loop body re-entered on a later iteration, or an `except` handler
/// reached partway through the `try` body it guards -- the caller drops
/// every currently-narrowed name this function reports from its working
/// narrowing state *before* checking/lowering starts, rather than only
/// from the kill's own source position onward. See
/// `crates/pycc_types/src/narrow.rs`'s module doc comment for the full
/// soundness rationale (loop re-entry and except-from-mid-try both
/// unsound under a single left-to-right source-order pass) and
/// `docs/decisions/` for the decision record narrowing D-199's
/// "never survives a reassignment" consequence to this conservative rule.
///
/// Mirrors exactly the set of statement kinds that route a bare-name
/// target through `check_assignment` in `pycc_types::lib`: `Assign`,
/// a *valued* `AnnAssign` (a value-less `Final` declaration never calls
/// `check_assignment` -- see that function's own doc comment), a
/// `ForRange`/`ForList` loop variable, and both names a list/set/dict
/// comprehension assignment binds (its own loop variable and its result
/// `target`). `DictSet`/`AttrSet` do not rebind a bare name (they mutate
/// a container/attribute the name still refers to) and are correctly
/// excluded. Recurses into every nested body this HIR can hold --
/// `If`/`While`/`ForRange`/`ForList`/`ForObject`/`Match`/`Try` -- so a
/// kill nested arbitrarily deep (e.g. `while ...: if flag: x = None`) is still
/// found. This match is intentionally exhaustive over every `HirStmt`
/// variant (no wildcard arm): adding a new statement kind that can kill a
/// binding forces this function to be updated rather than silently
/// under-reporting kills.
pub fn killed_names(body: &[HirStmt]) -> HashSet<String> {
    let mut killed = HashSet::new();
    collect_killed_names(body, &mut killed);
    killed
}

fn collect_killed_names(body: &[HirStmt], killed: &mut HashSet<String>) {
    for stmt in body {
        match stmt {
            HirStmt::Assign { target, .. } => {
                killed.insert(target.clone());
            }
            HirStmt::AnnAssign { target, value, .. } => {
                if value.is_some() {
                    killed.insert(target.clone());
                }
            }
            HirStmt::ForRange { var, body, .. } => {
                killed.insert(var.clone());
                collect_killed_names(body, killed);
            }
            HirStmt::ForList { var, body, .. } | HirStmt::ForObject { var, body, .. } => {
                killed.insert(var.clone());
                collect_killed_names(body, killed);
            }
            HirStmt::ListCompAssign { target, var, .. }
            | HirStmt::SetCompAssign { target, var, .. }
            | HirStmt::DictCompAssign { target, var, .. } => {
                killed.insert(target.clone());
                killed.insert(var.clone());
            }
            HirStmt::If { test, body, orelse } => {
                collect_named_expr_targets_in_expr(test, killed);
                collect_killed_names(body, killed);
                collect_killed_names(orelse, killed);
            }
            HirStmt::While { test, body } => {
                collect_named_expr_targets_in_expr(test, killed);
                collect_killed_names(body, killed);
            }
            HirStmt::Match { cases, .. } => {
                for case in cases {
                    // D-068 re-review of #780 (fourth round): a case's
                    // pattern can itself bind bare names (`case x:`, an
                    // `As`/`Sequence`/`Mapping`/`Class` capture) exactly
                    // like `check_pattern` routes through
                    // `check_assignment` in `pycc_types::lib`'s
                    // `check_match` -- these are kills too, not just
                    // statements inside `case.body`. Omitting them left
                    // `apply_kill_prescan` under-reporting a kill routed
                    // through a capturing `match` pattern inside a
                    // re-enterable body (see this function's own doc
                    // comment above).
                    collect_pattern_capture_names_as_killed(&case.pattern, killed);
                    collect_killed_names(&case.body, killed);
                }
            }
            HirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                collect_killed_names(body, killed);
                for handler in handlers {
                    // D-068 re-review of #780 (fourth round): `except ...
                    // as name:` binds `name` to the caught exception
                    // instance before the handler body runs -- the same
                    // kill of a bare name that a plain `Assign` performs
                    // (see the paired `check_assignment`/`bind` fix in
                    // `crates/pycc_types/src/exception.rs` and
                    // `crates/pycc_mir/src/stmt.rs`'s `Try` handler
                    // arm). Without this, `apply_kill_prescan` treated a
                    // handler's own `as` binding as invisible, letting a
                    // later re-entry into a body containing this handler
                    // (e.g. this `Try` nested inside a `while` loop)
                    // still read the pre-handler narrowed type.
                    if let Some(name) = &handler.name {
                        killed.insert(name.clone());
                    }
                    collect_killed_names(&handler.body, killed);
                }
                collect_killed_names(orelse, killed);
                collect_killed_names(finalbody, killed);
            }
            HirStmt::TryStar {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                // D-068 re-review of #780 (rebase onto #542's except* landing):
                // mirrors the plain `Try` arm above -- each `except* T as
                // name:` clause binds `name` to the caught `ExceptionGroup`
                // before the handler body runs, exactly the same bare-name
                // kill a plain `except ... as name:` performs (see the
                // paired MIR-side `kill_narrowing` fix in
                // `crates/pycc_mir/src/stmt.rs`'s `TryStar` handler arm).
                collect_killed_names(body, killed);
                for handler in handlers {
                    if let Some(name) = &handler.name {
                        killed.insert(name.clone());
                    }
                    collect_killed_names(&handler.body, killed);
                }
                collect_killed_names(orelse, killed);
                collect_killed_names(finalbody, killed);
            }
            HirStmt::ExprStmt(expr) => {
                collect_named_expr_targets_in_expr(expr, killed);
            }
            // #1244: a `del` unbinds a name rather than binding it, so it is
            // not a kill -- it narrows nothing and adds no definition. The
            // type checker tracks it separately (`deleted_names`). A slice
            // `del` (Part 2c of #1371) binds and unbinds nothing; a walrus
            // in its operands is refused by `stmt::del::lower_delete`.
            // #1291: a nested foreign import's name lives in
            // `HirModule::imports`, not `definition_spans`, exactly like a
            // top-level foreign import, so it is not a kill either;
            // recording it would make `reject_shadowed_foreign_imports`
            // report the import as shadowing itself.
            HirStmt::DictSet { .. }
            | HirStmt::AttrSet { .. }
            | HirStmt::Return(_)
            | HirStmt::Delete { .. }
            | HirStmt::DeleteSlice { .. }
            | HirStmt::DeleteAttr { .. }
            | HirStmt::ForeignImport { .. }
            | HirStmt::Raise { .. } => {}
        }
    }
}

/// D-068 re-review of #780 (fourth round): walks `pattern` for every bare
/// capture name it binds (recursively through `Sequence`/`SequenceStar`/
/// `Mapping`/`Class`/`Or`/`As`) and inserts each into `killed`. Mirrors
/// `pycc_types::collect_pattern_capture_names`'s own identical walk
/// (duplicated here rather than shared because `pycc_hir` has no dependency
/// on `pycc_types`, and `check_match`'s `check_pattern` routes every one of
/// these names through `check_assignment` -- see `collect_killed_names`'s
/// `Match` arm above and its own doc comment's inclusion criterion).
fn collect_pattern_capture_names_as_killed(pattern: &HirPattern, killed: &mut HashSet<String>) {
    match pattern {
        HirPattern::Wildcard
        | HirPattern::Literal(_)
        | HirPattern::Singleton(_)
        | HirPattern::NoneSingleton => {}
        HirPattern::Capture(name) => {
            killed.insert(name.clone());
        }
        HirPattern::Sequence(subs) | HirPattern::Or(subs) => {
            for sub in subs {
                collect_pattern_capture_names_as_killed(sub, killed);
            }
        }
        HirPattern::SequenceStar(subs, rest) => {
            for sub in subs {
                collect_pattern_capture_names_as_killed(sub, killed);
            }
            if let Some(rest) = rest {
                killed.insert(rest.clone());
            }
        }
        HirPattern::Mapping(pairs, rest) => {
            for (_, sub) in pairs {
                collect_pattern_capture_names_as_killed(sub, killed);
            }
            if let Some(rest) = rest {
                killed.insert(rest.clone());
            }
        }
        HirPattern::Class {
            positional,
            keyword,
            ..
        } => {
            for sub in positional {
                collect_pattern_capture_names_as_killed(sub, killed);
            }
            for (_, sub) in keyword {
                collect_pattern_capture_names_as_killed(sub, killed);
            }
        }
        HirPattern::As(inner, name) => {
            collect_pattern_capture_names_as_killed(inner, killed);
            killed.insert(name.clone());
        }
    }
}

/// D-068 review of #780/#774's interaction (blocker finding 2): walks `expr`
/// for every `HirExpr::NamedExpr { name, .. }` node at any depth and inserts
/// each `name` into `killed`. A bare walrus (`(x := None)`) is a reassignment
/// exactly like `HirStmt::Assign`, but it never appears as its own
/// `HirStmt` variant -- `lower_stmt`'s placement restriction (PEP 572) only
/// allows a `NamedExpr` inside a bare `HirStmt::ExprStmt`'s expression or an
/// `If`/`While` statement's `test`, so [`collect_killed_names`] must inspect
/// those two expression positions directly instead of relying on a
/// statement-level match arm the way every other kill kind above does.
///
/// Before this fix, `collect_killed_names` put a bare `ExprStmt` in a no-op
/// arm and never inspected `If`/`While`'s own `test`, so a re-enterable loop
/// body whose only kill of a narrowed name was a walrus (see
/// `docs/decisions/D-206-kill-prescan-for-re-enterable-narrowed-bodies.md`)
/// was invisible to the prescan: a read textually before the walrus within
/// the loop body was wrongly treated as still narrowed on every iteration
/// after the first, even though the *previous* iteration's walrus already
/// killed it. Mirrors `pycc_types::collect_named_expr_names_in_expr`'s own
/// identical walk (duplicated here rather than shared because `pycc_hir` has
/// no dependency on `pycc_types`, and this function only needs to collect
/// names, not also bind their inferred types the way that one does).
fn collect_named_expr_targets_in_expr(expr: &HirExpr, killed: &mut HashSet<String>) {
    match expr {
        HirExpr::NamedExpr { name, value } => {
            collect_named_expr_targets_in_expr(value, killed);
            killed.insert(name.clone());
        }
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::Super => {}
        HirExpr::ListPop { list } => {
            if let Some(receiver) = list.attr_expr() {
                collect_named_expr_targets_in_expr(receiver, killed);
            }
        }
        // #1254 (D-250): lowering refuses a walrus anywhere inside a
        // comprehension, and its loop variable is node-scoped, so a
        // comprehension kills no name of the enclosing scope.
        HirExpr::Comprehension(_) => {}
        HirExpr::Call { args, .. } => {
            for arg in args {
                collect_named_expr_targets_in_expr(arg, killed);
            }
        }
        // `BoolOp` walks both operands even though HIR lowering admits a
        // walrus only in the first: the kill is conservative.
        HirExpr::BinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            collect_named_expr_targets_in_expr(left, killed);
            collect_named_expr_targets_in_expr(right, killed);
        }
        // #1395: every part is walked, like `BoolOp`'s operands, although
        // lowering admits a walrus only in `test`: conservative.
        HirExpr::IfExp { test, body, orelse } => {
            collect_named_expr_targets_in_expr(test, killed);
            collect_named_expr_targets_in_expr(body, killed);
            collect_named_expr_targets_in_expr(orelse, killed);
        }
        // Every chain operand is walked, like `BoolOp`'s: conservative.
        HirExpr::CompareChain { first, links } => {
            collect_named_expr_targets_in_expr(first, killed);
            for link in links {
                collect_named_expr_targets_in_expr(&link.right, killed);
            }
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            collect_named_expr_targets_in_expr(operand, killed);
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    collect_named_expr_targets_in_expr(inner, killed);
                }
            }
        }
        HirExpr::ListLiteral(es)
        | HirExpr::ObjectList(es)
        | HirExpr::SetLiteral(es)
        | HirExpr::TupleLiteral(es) => {
            for e in es {
                collect_named_expr_targets_in_expr(e, killed);
            }
        }
        HirExpr::Subscript { base, index } => {
            collect_named_expr_targets_in_expr(base, killed);
            collect_named_expr_targets_in_expr(index, killed);
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            collect_named_expr_targets_in_expr(base, killed);
            for bound in [start, stop, step].into_iter().flatten() {
                collect_named_expr_targets_in_expr(bound, killed);
            }
        }
        HirExpr::ListAppend { list, value } => {
            if let Some(receiver) = list.attr_expr() {
                collect_named_expr_targets_in_expr(receiver, killed);
            }
            collect_named_expr_targets_in_expr(value, killed);
        }
        HirExpr::SetAdd { value, .. } => {
            collect_named_expr_targets_in_expr(value, killed);
        }
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                collect_named_expr_targets_in_expr(k, killed);
                collect_named_expr_targets_in_expr(v, killed);
            }
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            if let Some(receiver) = dict.attr_expr() {
                collect_named_expr_targets_in_expr(receiver, killed);
            }
            collect_named_expr_targets_in_expr(key, killed);
            collect_named_expr_targets_in_expr(default, killed);
        }
        HirExpr::AttrGet { base, .. } => collect_named_expr_targets_in_expr(base, killed),
        HirExpr::MethodCall { base, args, .. } => {
            collect_named_expr_targets_in_expr(base, killed);
            for arg in args {
                collect_named_expr_targets_in_expr(arg, killed);
            }
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            collect_named_expr_targets_in_expr(call, killed);
        }
        // #1411: `type(self)(args)` walks its arguments the same way.
        HirExpr::GenericClassInstantiate { args, .. } | HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                collect_named_expr_targets_in_expr(arg, killed);
            }
        }
        HirExpr::ExprCall { callee, args } => {
            collect_named_expr_targets_in_expr(callee, killed);
            for arg in args {
                collect_named_expr_targets_in_expr(arg, killed);
            }
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            collect_named_expr_targets_in_expr(call, killed);
            for (_, value) in keywords {
                collect_named_expr_targets_in_expr(value, killed);
            }
        }
    }
}

/// A compile-time-only import binding recorded by a module-level
/// `import`/`from ... import ...` statement resolved against `pycc_std`'s
/// registry (D-136/D-137). Mirrors `type_aliases`' side-table shape: an
/// import has zero runtime footprint of its own (no `HirStmt`/`HirItem` is
/// produced for it), it only makes a later name/attribute lookup resolve to
/// a stdlib registry entry.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportBinding {
    /// `import math` -- binds `math` as a module namespace marker; `import
    /// math as m` (Part 1 of #883, #962) binds `m` instead, so `local_name`
    /// is the alias when one is written and the module's canonical
    /// spelling (`pycc_std::module_name`) otherwise. `math` itself never
    /// carries a `Ty` -- only `<local_name>.<attr>` attribute access on
    /// this bound name resolves further, via `pycc_std::resolve_symbol`,
    /// and the lowered HIR always spells the result with the canonical
    /// module name (`"math.sqrt"`), never the alias. A `pycc_std` module
    /// name itself never carries a `Ty` -- unlike
    /// [`ImportBinding::Foreign`], whose bound name *is* a value.
    Module {
        local_name: String,
        module: pycc_std::StdModule,
    },
    /// `from math import sqrt` -- binds `sqrt` directly to the resolved
    /// registry symbol, as if it were a fixed, non-inferred `Ty`/signature
    /// (the alias table from PR-13 is the closest existing precedent).
    Symbol {
        local_name: String,
        module: pycc_std::StdModule,
        symbol: pycc_std::StdSymbol,
    },
    /// `from geometry import Point` where `geometry` is a project file the
    /// driver resolved and lowered first (#898, D-222): binds `local_name`
    /// to the top-level definition of the same name in `module_path`, the
    /// display path of the module that *defines* it (never of a module that
    /// merely re-exports it -- a re-export chain is collapsed at binding
    /// time, so one hop always reaches the definition). Like the two
    /// stdlib variants it has zero runtime footprint: after
    /// `program::link` concatenates every module into one program the
    /// definition itself is in scope under the same name, so the binding
    /// only records provenance for later re-exports and diagnostics.
    Project {
        local_name: String,
        module_path: String,
        kind: ProjectBindingKind,
    },
    /// `import numpy` where `numpy` is neither a project module nor a
    /// `pycc_std` one (Part 1 of #1026): binds `local_name` to an opaque
    /// CPython module object, `Ty::Object`. Unlike every other variant
    /// this one is *not* compile-time-only -- the bound name is a real
    /// runtime value, a `PyObject *` the generated `Py_mod_exec` slot
    /// obtains from `pycc_ext_obj_import(module_path)`.
    ///
    /// `from` is `None` for that `import` form. For one name of an
    /// unaliased `from itertools import product` -- at top level (#1278) or
    /// nested in a module-level `if`/`try` block (#1383) -- it is `Some`, and the bound object is the module's attribute of that name,
    /// fetched with CPython's own `IMPORT_FROM` semantics by
    /// `pycc_ext_obj_import_from`; `local_name` is the name, and
    /// `module_path` stays the module, so every gate that classifies by the
    /// module root (the lock, the interop policy, `I0403`) keeps reading it.
    /// Under `pycc build --ext --foreign-relative-imports` (#1366) the entry
    /// module's relative `from .sib import x` is foreign too: `module_path`
    /// is then the name as written *without* its leading dots (`""` for
    /// `from . import x`), and [`FromImport::level`] carries the dots. Those
    /// root gates never see such a binding, because the flag requires
    /// `--ext`, which skips all three.
    ///
    /// A plain dotted import (#1381, Part 3 of #1138) keeps the whole
    /// dotted name in `module_path` with `from` `None`. Unaliased, `import
    /// a.b` imports `a.b` and binds the *root* `a`, so `local_name` is `a`;
    /// aliased, `import a.b as c` binds the *leaf* `a.b` to `c`. Which of the
    /// two a binding is follows from these fields alone --
    /// [`foreign_binds_root`] -- because `pycc_hir` refuses the one spelling
    /// they cannot tell apart, `import a.b as a`. Both forms are fetched by
    /// `pycc_ext_obj_import_dotted`.
    ///
    /// `site` says where the import runs; see [`ForeignImportSite`].
    ///
    /// `span` is the `import` statement's own source range. Every other
    /// variant is compile-time-only and is never the subject of a
    /// diagnostic of its own, but this one can be: a native build may refuse
    /// it (`I0403`), and a module that binds the same local name twice
    /// refuses that too (`C0001`). Both diagnostics must point at the
    /// import statement, and the import side table carries no position
    /// otherwise -- an item position counts items, not bytes, so it cannot
    /// stand in for one (PR 1c of #1080 review round 4).
    Foreign {
        local_name: String,
        module_path: String,
        from: Option<FromImport>,
        site: ForeignImportSite,
        span: Span,
    },
}

/// The `from` half of an [`ImportBinding::Foreign`] (#1278): one name of an
/// unaliased `from <foreign module> import a, b` statement, at top level
/// (#1278) or nested in a module-level `if`/`try` block (#1383).
///
/// Every binding of one statement carries the same `fromlist`, the
/// statement's whole name list in source order, because CPython's
/// `IMPORT_NAME` receives all of it before any `IMPORT_FROM` runs, and
/// `index` is this binding's own position in it. Codegen needs the list for
/// the import call; the driver's per-statement diagnostics (`I0402`,
/// `I0403`) render it and report once per statement, on the binding whose
/// `index` is `0` (see [`FromImport::opens_statement`]).
///
/// `level` is the statement's count of leading dots (#1366): `0` for an
/// absolute `from X import n`, and `1` or more for a relative one, which
/// only the entry module of a `pycc build --ext --foreign-relative-imports`
/// binds this way. The module name the binding records never includes the
/// dots; [`FromImport::spelled_module`] and [`FromImport::spelled_object`]
/// render them back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FromImport {
    pub name: String,
    pub fromlist: Vec<String>,
    pub index: usize,
    pub level: u32,
}

impl FromImport {
    /// Whether this is the first name of its statement.
    ///
    /// A statement-level diagnostic about a foreign import is reported on
    /// that binding alone, so `from tkinter import Tk, Label` gives one
    /// `I0403` rather than two. The index is the binding's own, not a
    /// comparison with its neighbour: the linked program's import table
    /// concatenates every file's, and two files can hold byte-identical
    /// statements at the same span, each owed its own diagnostic.
    pub fn opens_statement(&self) -> bool {
        self.index == 0
    }

    /// The module as the statement spells it (#1366): `module_path` behind
    /// `level` dots, so `itertools`, `.sib`, `..a.b`, or `.` for `from .
    /// import x`.
    pub fn spelled_module(&self, module_path: &str) -> String {
        format!("{}{module_path}", ".".repeat(self.level as usize))
    }

    /// The object this binding binds, spelled as a dotted path (#1366):
    /// `itertools.product`, `.sib.x`, `..a.b.c`, or `.sib` for `from .
    /// import sib`. The separating dot is written only when there is a module
    /// name to separate the attribute from.
    pub fn spelled_object(&self, module_path: &str) -> String {
        let module = self.spelled_module(module_path);
        if module_path.is_empty() {
            format!("{module}{}", self.name)
        } else {
            format!("{module}.{}", self.name)
        }
    }
}

/// Whether a foreign binding is the one a statement-level diagnostic about
/// its import statement is reported on: every `import X` binding (each
/// alias of `import a, b` is its own import, #1280), and the first name of
/// a `from X import a, b` (#1278).
pub fn opens_foreign_statement(from: Option<&FromImport>) -> bool {
    from.is_none_or(FromImport::opens_statement)
}

/// The source form of a foreign import as diagnostics quote it:
/// `import numpy`, `import numpy as np` (#1291), `import xml.dom` or
/// `import xml.dom as d` (#1381), or `from tkinter import Tk, Label`
/// (#1278), or `from ..lexer import Token` for a relative one (#1366).
///
/// A plain import is aliased exactly when its `local_name` differs from
/// the root segment of `module_path`: unaliased `import a` binds `a`, and
/// unaliased `import a.b` binds its root `a` ([`foreign_binds_root`]);
/// `import a.b as a`, the one alias equal to its root, is refused before
/// any diagnostic quotes it. A from-import ignores `local_name`.
pub fn foreign_import_statement(
    local_name: &str,
    module_path: &str,
    from: Option<&FromImport>,
) -> String {
    match from {
        None => {
            let root = module_path.split('.').next().unwrap_or(module_path);
            if local_name == root {
                format!("import {module_path}")
            } else {
                format!("import {module_path} as {local_name}")
            }
        }
        Some(from) => format!(
            "from {} import {}",
            from.spelled_module(module_path),
            from.fromlist.join(", ")
        ),
    }
}

/// Whether an [`ImportBinding::Foreign`] with these fields is the root
/// binding of an unaliased plain dotted import (#1381): `import a.b`, which
/// imports `a.b` but binds the root module `a` to `local_name` `a`. Every
/// other plain import binds the module `module_path` names -- `import a`,
/// and the leaf of an aliased `import a.b as c` -- and a from-import binds
/// an attribute.
///
/// The answer is exact only because two invariants hold. `pycc_hir`
/// refuses `import a.b as a` (`import::lower_import_alias`), the one
/// spelling whose leaf binding would read as a root binding here; and no
/// pass renames a foreign binding's `local_name` (`program::link` and the
/// `pycc_types` position remaps rebuild it field for field). A pass that
/// starts renaming foreign locals must carry the root/leaf distinction
/// explicitly instead.
pub fn foreign_binds_root(local_name: &str, module_path: &str, from: Option<&FromImport>) -> bool {
    from.is_none()
        && module_path
            .split_once('.')
            .is_some_and(|(root, _)| root == local_name)
}

/// The module a plain foreign import binds (#1381): the root `a` for the
/// root binding of `import a.b` ([`foreign_binds_root`]), `module_path`
/// otherwise. This, not `module_path`, is the bound object's identity:
/// `import os` and `import os.path` both bind the module `os`.
pub fn foreign_bound_module<'a>(
    local_name: &str,
    module_path: &'a str,
    from: Option<&FromImport>,
) -> &'a str {
    match module_path.split_once('.') {
        Some((root, _)) if foreign_binds_root(local_name, module_path, from) => root,
        _ => module_path,
    }
}

/// What a foreign binding binds, as diagnostics name it: `the CPython module
/// `numpy`` for `import numpy`, `the CPython object `itertools.product``
/// for `from itertools import product` (#1278), `the CPython object
/// `.sib`` for a relative `from . import sib` (#1366). Callers pass the
/// bound module ([`foreign_bound_module`]), so `import a.b` names `a`
/// (#1381).
pub fn foreign_bound_object(module_path: &str, from: Option<&FromImport>) -> String {
    match from {
        None => format!("the CPython module `{module_path}`"),
        Some(from) => format!("the CPython object `{}`", from.spelled_object(module_path)),
    }
}

/// Where an [`ImportBinding::Foreign`] import runs in its module body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignImportSite {
    /// A module-level `import` statement. The payload is the number of
    /// `HirItem`s the module statements *preceding* this import produced.
    /// `HirModule::imports` is a side table with no position of its own,
    /// and a top-level `import` statement produces no `HirItem`, so without
    /// this index the import's place in the module body would be lost and
    /// the generated import call would have to be hoisted ahead of every
    /// statement -- which CPython does not do and which D-244 rule 3 does
    /// not permit. The index is module-local when `module::lower_module`
    /// records it, is rebased onto the linked program's item list by
    /// `program::link`, and is where `pycc_mir`'s `splice_foreign_imports`
    /// inserts a `MirItem::ForeignImport`.
    Item(usize),
    /// An `import` (#1291) or unaliased foreign `from ... import ...`
    /// (#1383) statement nested in a module-level `if`/`try` block; a
    /// from-import's bindings carry their [`FromImport`]. It runs as the `HirStmt::ForeignImport` statement lowered
    /// at its own position inside the block, so it is never spliced; the
    /// table entry exists so the name reaches the driver's lock, interop
    /// policy (`I0402`) and native-build (`I0403`) gates, and the
    /// position-blind passes that seed every foreign name as a
    /// `Ty::Object` module global.
    ///
    /// `optional` is true when the import is *optional* (#1290): it sits,
    /// at any depth, in the body of a `try` (or `try`/`except*`) that is
    /// at module level or nested only in module-level `if`/`try` blocks,
    /// with a handler that catches a failed import -- a bare
    /// `except:`, or a handler naming `ImportError`, `ModuleNotFoundError`
    /// or `Exception`, alone or in a tuple. A handler, `else` or `finally`
    /// body inherits the enclosing guard rather than creating one. The lock
    /// records an optional root under `optional-roots` and does not refuse
    /// it when no installed distribution owns it (D-249, amended by #1290).
    Block {
        /// Whether a qualifying `try` guards this import (see above).
        optional: bool,
    },
}

/// Which kind of top-level definition an [`ImportBinding::Project`] names
/// in its origin module -- the four binding kinds a module's top level can
/// hold in this compiler's flat namespace model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectBindingKind {
    /// A `class` statement. The importing module's lowering also receives a
    /// clone of the class and its whole MRO so annotations and bases can
    /// resolve it; `module::lower_module` strips those clones again before
    /// building its `HirModule`, so the linked program defines each class
    /// exactly once.
    Class,
    /// A top-level `def`.
    Function,
    /// A name bound by a top-level statement (`X = 1`, a loop variable, a
    /// walrus target, ...) -- exactly the set `top_level_bound_names`
    /// reports.
    Variable,
    /// A `type X = ...` alias or its legacy `X: TypeAlias = ...` spelling;
    /// the alias table entry is copied into the importer too.
    TypeAlias,
}

/// Every bare name a module's top-level statements bind at runtime, in no
/// particular order -- the `killed_names` walk applied to the module's
/// `HirItem::TopLevelStmt`s only (a top-level `def` is a function, not a
/// variable, and is looked up separately by `import::lower_import_stmt`).
/// This is the `ProjectBindingKind::Variable` lookup table for a project
/// import (#898).
pub fn top_level_bound_names(items: &[HirItem]) -> HashSet<String> {
    let mut bound = HashSet::new();
    for item in items {
        if let HirItem::TopLevelStmt(stmt) = item {
            collect_killed_names(std::slice::from_ref(stmt), &mut bound);
        }
    }
    bound
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirModule {
    pub items: Vec<HirItem>,
    /// Compile-time-only name-to-`Ty` bindings from a `type X = <expr>`
    /// statement or a legacy `X: TypeAlias = <expr>` annotated assignment
    /// (D-135). Populated in source order by `module::lower_module` as it walks the
    /// module body. Neither form lowers to any `HirItem`/`HirStmt` of its
    /// own -- the alias has zero HIR/MIR/codegen/runtime footprint, and this
    /// field exists purely so a later annotation naming the alias resolves
    /// to the same `Ty` (see `annotation_to_ty`'s alias-table lookup).
    pub type_aliases: Vec<(String, Ty)>,
    /// Import bindings (D-136/D-137, #898 for `ImportBinding::Project`, and
    /// Part 1 of #1026 for `ImportBinding::Foreign`), populated in source
    /// order by `module::lower_module` exactly like `type_aliases`. A
    /// module-level `import`/`from ... import ...` statement records its
    /// bindings here, and so does a foreign `import` (#1291) or unaliased
    /// foreign `from ... import ...` (#1383) nested in a module-level
    /// `if`/`try` block ([`ForeignImportSite::Block`]). Every other nested
    /// import, including a nested from-import the driver did not answer
    /// foreign, still reaches `lower_stmt`'s `C0001` catch-all.
    pub imports: Vec<ImportBinding>,
    /// Class name -> declared shape (attribute slots in first-`__init__`-
    /// assignment order, method table) (D-154, Part 1 of #375). Populated by
    /// `class::lower_class` as `module::lower_module` walks the module body, in
    /// source order, mirroring `type_aliases`/`imports`'s own shape: a class
    /// definition has no `HirItem`/`HirStmt` footprint of its own (unlike a
    /// top-level function) -- only its individual methods do, each lowered
    /// into `items` as an ordinary mangled `HirItem::Function` (see
    /// `class::lower_class`'s own doc comment for the mangling scheme and
    /// the reasoning for not adding a dedicated `HirItem::ClassDef` variant).
    pub class_defs: Vec<(String, HirClassDef)>,
    /// Provenance for the builtin exception hierarchy (Part 1 of #541,
    /// D-188): `true` exactly when *this* lowering pass seeded every
    /// `BUILTIN_EXCEPTION_CLASSES` entry into `class_defs`, and `false`
    /// for every module whose classes are all user-authored.
    ///
    /// Seeding is all-or-nothing and its shadow gate guarantees no user
    /// top-level binding of any builtin exception name survives alongside it,
    /// so this single flag plus `is_builtin_exception_class` identifies the
    /// synthetic entries exactly -- see `pycc_types`'s `bind_classes`.
    /// Provenance is recorded here rather than re-derived downstream
    /// because *no* property of a `HirClassDef`'s own shape is a sound
    /// proxy for who produced it: a user can author a class that is
    /// byte-for-byte identical to a synthetic one.
    pub seeded_builtin_exception_classes: bool,
}
