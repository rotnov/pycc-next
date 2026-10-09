//! Module-level lowering: the single left-to-right walk over a module's
//! top-level statements that dispatches each one to the alias, import,
//! class, function, or statement lowering in the sibling modules and
//! assembles the resulting `HirModule`.
//!
//! Extracted from `lib.rs` per AGENTS.md's file-decomposition rule (issue
//! #867): the crate root keeps the HIR data types and the diagnostic
//! constructors; the walk itself lives here. `lib.rs` re-exports
//! `lower_all` and `lower_checked` so the `pycc_hir::lower_checked` path is
//! unchanged.
//!
//! #898 (D-222) splits the walk from the program-level phases that used to
//! follow it: `lower_module` is the per-module walk (against the driver's
//! `ResolvedImports` answers for the module's project imports), and
//! `program::link` + `program::finalize` combine any number of lowered
//! modules into one program and run the post-loop phases (exception type
//! tags, `Exception.__init__`). `lower_all` is the single-file composition
//! of the two and is byte-identical to what it produced before the split.
//!
//! Part 2 of #864 (#867, D-219): the walk collects one diagnostic per
//! failing top-level item instead of stopping at the first. A failing item
//! is skipped as a unit -- a `def` aborts only that function, a failing
//! method aborts its whole class -- and lowering continues with the next
//! item. Some `C0001` shapes are *cascades* of an earlier skipped item rather
//! than independent gaps: a bare-name or bare-container annotation that
//! names a class, type alias, or import which failed to lower, and a base-class
//! reference to one (whether its message says "unknown class" or, since
//! Part 1 of #1283, "builtin type").
//! The lowering source suppresses those silently through the "poisoned
//! bindings" set kept by `lower_module` (see `poisonable_names` and
//! `cascade_name`); everything else it produces is reported. The suppression
//! covers the lowering source only: the #944 per-item enum-call scan (D-233,
//! `class::enum_call`) runs after every item whatever its outcome, so a
//! cascade-silenced item can still report a true enum-call `C0001` of its
//! own. HIR failures still stop the pipeline before the type
//! checker (`src/frontend.rs`), so no partial module is ever type-checked.

mod lowered;
pub(crate) mod own_foreign;
mod poison;
mod type_var;

pub use lowered::LoweredModule;
use lowered::strip_imported;

pub(crate) use poison::{
    bare_container_annotation_message, builtin_base_message, cascade_name, poisonable_names,
    unknown_annotation_name_message, unknown_base_message,
};

use crate::expr::keyword_bind::SignatureTable;
use crate::import::{FuturePosition, ResolvedImports, future_prologue_len};
use crate::{
    ForeignImportSite, HirClassDef, HirItem, HirModule, ImportBinding, Ty,
    builtin_exception_class_defs, class, dunder_name, exception, import_local_name, killed_names,
    lower_function, lower_import_stmt, lower_legacy_type_alias_ann_assign, lower_type_alias_stmt,
    program, stmt, unsupported,
};
use pycc_ast::{ModModule, Stmt};
use pycc_diag::{Diagnostic, Span};

/// Lowers a parsed module into the HIR subset implemented by this pycc
/// version. Syntactically valid Python outside that subset returns `C0001`
/// with the unsupported node's source span instead of panicking.
///
/// First-diagnostic view of [`lower_all`] for the crate's many test, bench,
/// and downstream callers that consume a single `Diagnostic` (D-217's
/// `parse`/`parse_all` precedent): the `Err` is exactly `lower_all`'s first
/// collected diagnostic. That is the first *lowering* diagnostic,
/// byte-identical to what this function reported before per-item
/// collection landed (D-219), unless an enum-call `C0001` scanned from an
/// earlier, successfully lowered item precedes it in loop order (#944,
/// D-233 decision 4). The `.expect` follows
/// the crate's documented coverage convention (`import.rs`): `lower_all`'s
/// `Err` is never empty by construction, and the panic path lives in
/// libcore, adding no in-crate region.
pub fn lower_checked(module: &ModModule) -> Result<HirModule, Diagnostic> {
    lower_all(module).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .next()
            .expect("lower_all's Err is never empty by construction")
    })
}

/// The tables `lower_module` builds up as it walks a module's top-level
/// statements, in source order. Each is a `Vec` rather than a map so the
/// lookups every later item performs see earlier items in a stable order.
struct ModuleState<'a> {
    aliases: Vec<(String, Ty)>,
    imports: Vec<ImportBinding>,
    class_defs: Vec<(String, HirClassDef)>,
    // #585: parallel to `class_defs`, but keeps each user-authored class's
    // original `StmtClassDef` (borrowed from the module, so it outlives this
    // whole pass) instead of its already-lowered `HirClassDef`. A synthetic
    // builtin-exception class seeded into `class_defs` has no such AST node
    // and is simply never present here -- `class::lower_class` treats that
    // absence as "nothing further to validate", matching how this crate
    // already handles bases it cannot introspect elsewhere.
    class_asts: Vec<(String, &'a pycc_ast::StmtClassDef)>,
    // #1368: one `__slots__` row per user class in `class_defs` (lowered
    // here or copied in by an import); see `class::slots`.
    class_slots: Vec<class::slots::ClassSlotsRow>,
    items: Vec<HirItem>,
    // #898: positions in `class_defs`/`aliases` that a project import
    // copied in from another module so this module's annotations and
    // bases can resolve them. Stripped again before the `HirModule` is
    // built: the linked program defines each class and alias exactly once,
    // in the module that authored it.
    imported_class_indices: Vec<usize>,
    imported_alias_indices: Vec<usize>,
    // #898: every top-level definition this module makes, with its span,
    // so `program::link` can report a cross-module name collision at the
    // later definition. Names may repeat (a variable rebound twice).
    definition_spans: Vec<(String, Span)>,
    /// #1485: each `except ImportError` fallback name of a module-level `try`.
    fallback_groups: Vec<crate::import::FallbackGroup>,
    /// The module's keyword-bindable signature table (Part 1 of #884,
    /// #1125). Collected from the whole module body *before* the item loop
    /// so a keyword call written above its own `def` binds just as well as
    /// one written below it, and deliberately lowering-internal: the table
    /// itself never reaches [`LoweredModule`] or `program::link`; only its
    /// final `object_receivers` bit is published (#1425).
    signatures: SignatureTable,
    /// Whether the module body binds `staticmethod` anywhere (#1345),
    /// computed once before the item loop and handed to every class.
    staticmethod_rebound: bool,
    /// The module-level `T = TypeVar("T")` declarations so far (#1394),
    /// handed to every class for its `Generic[...]` base.
    type_vars: Vec<String>,
}

/// Lowers every top-level item of a parsed module, collecting one
/// diagnostic per failing item (in source order) plus, per item, one
/// `C0001` for every call to an enum class that the scan can attribute
/// (#921, #944, `class::enum_call`, D-233 -- a call the scan's documented
/// limits suppress, such as one whose name another module-level statement
/// also binds, lowers `Ok` here and is caught by the type checker's
/// span-less guard instead), and skipping a failing item; the `Err` is
/// never empty (D-219, Part 2 of #864). The single-file
/// entry: exactly `lower_module` with no project imports answered,
/// followed by `program::finalize` -- the same phases in the same order as
/// before #898, so the result is byte-identical.
pub fn lower_all(module: &ModModule) -> Result<HirModule, Vec<Diagnostic>> {
    // No module name: this entry is the in-crate/test single-file path, and
    // the driver -- the only caller that knows whether the file is an entry
    // module and under which name -- goes through `lower_module` directly.
    let lowered = lower_module(module, &ResolvedImports::default(), None)?;
    program::finalize(lowered.hir)
}

/// The per-module walk (#898): lowers every top-level item against the
/// driver's answers for the module's project imports, collecting one
/// diagnostic per failing item (in source order) and skipping that item,
/// then -- for every item, lowered or skipped -- one `C0001` per call to an
/// enum class inside it that no scope-local binding shadows (#921, #944,
/// `class::enum_call`; the second per-item collection source, appended
/// right after the item's own diagnostic so the list stays in loop order,
/// D-233 amending D-219); the `Err` is never empty (D-219). The result
/// still needs
/// `program::link` (even for a single module) and `program::finalize`
/// before it is a complete program: the exception type tags and the
/// synthetic `Exception.__init__` are program-wide and assigned there.
///
/// Type aliases (D-135) are resolved in a single left-to-right pass: a
/// `type X = <expr>` or legacy `X: TypeAlias = <expr>` statement is
/// evaluated and recorded into `aliases` as soon as it is reached, so it is
/// visible to every later statement's annotations (including a later
/// function's parameter/return annotations and later top-level
/// `AnnAssign`s) but not to any earlier one -- matching this compiler's
/// existing single-pass, source-order lowering model instead of
/// introducing hoisting.
///
/// Cascade suppression ("poisoned bindings", D-219): every item is lowered
/// first and its failure classified afterwards. When an item fails, the
/// class, type-alias, or project-import names it would have bound
/// (`poisonable_names`) are recorded as poisoned; when a later item fails
/// with one of the cascade-shaped `C0001`s (`cascade_name`) naming a
/// poisoned name, that item's own lowering diagnostic is dropped *silently*
/// (the post-item enum-call scan still runs on it, D-233 decision 3, so a
/// call to another, valid enum class inside it is still reported) and its
/// own poisonable names are recorded too, so `class B(A)`
/// after a skipped `A` silences a following `class C(B)`. A later item
/// that binds a poisoned name and lowers successfully un-poisons it.
/// Nothing before the first failing item is ever skipped, and that item's
/// diagnostic is pushed unconditionally (the set is still empty). Since
/// #944 (D-233 decision 4) the first collected diagnostic is not always
/// that item's own: an enum-call `C0001` scanned from an earlier,
/// successfully lowered item precedes it in loop order. The first
/// *lowering* diagnostic is still byte-identical to the pre-#867 single
/// diagnostic (D-217 rule 2).
pub fn lower_module(
    module: &ModModule,
    resolved: &ResolvedImports<'_>,
    module_name: Option<&str>,
) -> Result<LoweredModule, Vec<Diagnostic>> {
    let mut state = ModuleState {
        aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
        class_asts: Vec::new(),
        class_slots: Vec::new(),
        items: Vec::with_capacity(module.body.len()),
        imported_class_indices: Vec::new(),
        imported_alias_indices: Vec::new(),
        definition_spans: Vec::new(),
        fallback_groups: Vec::new(),
        signatures: SignatureTable::collect(&module.body, |import| {
            resolved.resolves_to_project_module(import.range)
        }),
        // No import is known yet, so only the unaliased `TYPE_CHECKING`
        // guards are recognized; a binding under an aliased guard counts,
        // which can only refuse a program, never admit one.
        staticmethod_rebound: class::foreign_static::binds_name(&module.body, &[], "staticmethod"),
        type_vars: Vec::new(),
    };
    state
        .signatures
        .inherit_container_method_names(resolved.container_method_names().iter().copied());
    // Issue #1425: a direct dependency can hold a CPython object, so any
    // receiver here may be one -- for the whole module, as below for an own
    // foreign import (#1482).
    if resolved.object_receivers() {
        state.signatures.admit_object_receivers();
    }
    // D-258 (#1397): an `ext` module records the marker entry that tells
    // `annotation_to_ty` to lower `Any`, `object` and an object container to
    // `Ty::Object` (see `func::EXT_MODULE_MARKER`). Recorded as imported, so
    // `strip_imported` keeps it out of `HirModule::type_aliases`.
    if resolved.ext_module() {
        // Issue #1095: any receiver in such a module may be an object.
        state.signatures.admit_object_receivers();
        state.imported_alias_indices.push(state.aliases.len());
        state
            .aliases
            .push((crate::func::EXT_MODULE_MARKER.to_string(), Ty::Object));
    }
    // Issue #1482: an own foreign import, top-level or in a module-level
    // `if`/`try`, admits for the whole module too, so a `def` written above
    // `import gc` reads `gc` as CPython does when it runs. Skipped when the
    // module is already admitted.
    if !(resolved.object_receivers() || resolved.ext_module())
        && own_foreign::binds_foreign_import(module, resolved, &state.imports)
    {
        state.signatures.admit_object_receivers();
    }
    let container_method_names = state.signatures.container_method_names().clone();
    // Part 1 of #541 (extending D-173): give the builtin exception
    // hierarchy a real presence in the class table, seeded *before* any
    // user statement is lowered so a user class can inherit from one
    // (`class MyError(ValueError):`) exactly as it inherits from a user
    // base. Two gates, both of which must pass:
    //
    // * The module must actually *reference* a builtin exception name. Every
    //   entry in `class_defs` costs the per-item work below (the projected
    //   class slice, the name-collision checks) and the per-function class
    //   binding in `pycc_types`, and a module that never names a builtin
    //   exception cannot observe the difference -- see
    //   `exception::module_references_builtin_exception_name`.
    // * The module's own top level must not *bind* any builtin exception name.
    //   That gate is all-or-nothing, so every existing name-collision check
    //   below applies to the synthetic definitions with no exemption -- see
    //   `exception::shadowed_builtin_exception_name`. Both gates are
    //   whole-module AST scans decided here, before the loop, so a
    //   shadowing class that later fails to lower still counts as a shadow.
    let shadowed_builtin_exception_name = exception::shadowed_builtin_exception_name(module);
    let seeded_builtin_exception_classes =
        exception::module_references_builtin_exception_name(module)
            && shadowed_builtin_exception_name.is_none();
    if seeded_builtin_exception_classes {
        state.class_defs.extend(builtin_exception_class_defs());
    }
    // #1369: an `assert` raises the builtin `AssertionError`, which a module
    // that withheld seeding does not have. Refused once, naming the binding
    // responsible, as the diagnostic of the first top-level item containing
    // an `assert` -- see `exception::first_assert_statement_range`. It is
    // emitted in the loop below *in place of* that item's own lowering
    // rather than returned from here, so D-217 rule 2 holds: every earlier
    // item is still lowered and reports first, and the loop keeps going
    // after it (#867/D-233's per-item collection). Replacing, not adding to,
    // the item's lowering keeps its other diagnostic from being a
    // consequence of the missing class. The item bound nothing, so its
    // names are poisoned like any other failing item's.
    // An `assert` the `TYPE_CHECKING` fold discards is never lowered, so it
    // is never refused; the scan sees the whole module's stdlib imports, as
    // the `__name__` scans below do.
    let mut assert_refusal: Option<(usize, Diagnostic)> = shadowed_builtin_exception_name
        .as_ref()
        .and_then(|shadowed| {
            let imports = dunder_name::scan_imports(module, &state.imports);
            module.body.iter().enumerate().find_map(|(index, stmt)| {
                exception::first_assert_statement_range(std::slice::from_ref(stmt), &imports).map(
                    |range| {
                        let diagnostic = unsupported(
                            format!(
                                "an `assert` statement needs the builtin `AssertionError`, \
                                 which is unavailable because this module binds the builtin \
                                 exception name `{shadowed}` at top level"
                            ),
                            range,
                        );
                        (index, diagnostic)
                    },
                )
            })
        });
    // W0 of #882 (#1156): the compiler-provided `__name__` binding, pushed
    // into the still-empty item list so it is the module's *first* top-level
    // statement -- an ordinary `str` global every downstream pass already
    // knows how to compile, so no new HIR/MIR/type/codegen node exists for
    // it. `dunder_name` owns THE RULE and both seeding gates; `module_name`
    // is `None` for every module the driver did not name (a non-entry module
    // of a multi-file program, and the single-file `lower_all` path), which
    // withholds the seed. Deliberately *not* recorded in `definition_spans`:
    // that table drives `program::link`'s cross-module collision check, and
    // registering a synthetic definition there would report a `C0001` against
    // a statement no user wrote. Nothing else depends on that exclusion: a
    // user binding in *any* module withholds the seed program-wide, through
    // `LoweredModule::mentions_dunder_name`, so a seed and a user binding of this
    // name never coexist in a linked program.
    // Both scans run here, before the loop below has appended this module's
    // own `import` statements, so they take the driver's answers
    // (`state.imports`) and reconstruct the module's own stdlib imports
    // themselves -- see `dunder_name::scan_imports`, which exists so an
    // aliased `if t.TYPE_CHECKING:` folds in the scans exactly as it folds in
    // `lower_stmt`. One slice for both halves of THE RULE keeps the entry gate
    // and the dependency gate from disagreeing about which guarded bodies are
    // dead.
    if let Some(item) = dunder_name::seed_item(module, module_name, &state.imports) {
        state.items.push(item);
    }
    let mentions_dunder_name = dunder_name::mentions_dunder_name(module, &state.imports);
    // Seeded at the *front* so every lookup below (base resolution,
    // annotation projection, the name-collision checks) sees them, then
    // rotated to the back once lowering finishes so `class_defs` still
    // opens with the module's own classes in source order.
    let synthetic_class_count = state.class_defs.len();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    // Small and searched linearly; insertion order keeps tests deterministic.
    let mut poisoned: Vec<String> = Vec::new();
    // D-229: CPython allows a `from __future__ import ...` only in the
    // module's prologue (docstring, then future imports); every later
    // statement is `Body`, and a future import there is an `L0001`.
    let prologue_len = future_prologue_len(&module.body);
    // #944: the module's own enum classes, known before the loop so a call
    // inside a `def` that precedes `class Color(Enum):` is still scanned
    // against it, and the module frame -- every name the module body binds
    // directly, which is never a `class`/`def`/`import` name -- so a plain
    // module-level `Color = 1` keeps its `T0021` (see `class::enum_call`).
    // A `TYPE_CHECKING`-guarded module-level body (#790) binds nothing at
    // runtime, so the frame skips it -- recognized against the imports known
    // *before* the loop (the driver's answers, never this module's own
    // `import` statements, which the loop has not lowered yet): the bare
    // `TYPE_CHECKING` and `typing.TYPE_CHECKING` spellings resolve without
    // any import binding, while an aliased `t.TYPE_CHECKING` guard does
    // not, and its body's bindings stay in the frame (limit (vi) in
    // `class::enum_call`, over-suppression only).
    let syntactic_enum_classes = class::enum_call::syntactic_enum_class_names(&module.body);
    // A name that another module-level `def`/`class`/`import`/`type`
    // statement also binds is a collision the class item reports itself;
    // the scan never claims it (limit (vii) in `class::enum_call`), so
    // `def Color()` / `Color()` / `class Color(Enum)` yields the collision
    // diagnostic alone. Like the frame below, it is computed on the first
    // item whose enum-name set is non-empty (it is a pass over every
    // module-level binding, quadratic in their count).
    let mut rebound_names: Option<Vec<String>> = None;
    // Built on the first item whose name set is non-empty, never for a
    // module that defines and imports no enum class (D-233 decision 1: the
    // common module pays for no part of this diagnostic, and the frame is
    // a walk of every live module-level statement). `state.imports` only
    // grows (`append` in `lower_top_level_item`), so the pre-loop slice is
    // exactly the driver's answers whichever item first needs the frame.
    let pre_loop_imports = state.imports.len();
    let mut module_frame: Option<Vec<String>> = None;
    // #1380: the items that failed (reported or cascade-skipped), so the
    // carrier scan below never reports a second diagnostic for one.
    let mut failed_items: Vec<usize> = Vec::new();
    // #1380: where each item's diagnostics begin, so the carrier scan's
    // diagnostics can be spliced back into per-item source order.
    let mut item_starts: Vec<usize> = Vec::with_capacity(module.body.len());
    for (index, stmt) in module.body.iter().enumerate() {
        item_starts.push(diagnostics.len());
        let position = if index < prologue_len {
            FuturePosition::Prologue
        } else {
            FuturePosition::Body
        };
        // #1418: in an `ext` module, a `NotImplemented` outside an admitted
        // `return NotImplemented` refuses its item the same way.
        let outcome = match assert_refusal.take_if(|(refused, _)| *refused == index) {
            Some((_, diagnostic)) => Err(diagnostic),
            None => match resolved
                .ext_module()
                .then(|| crate::not_implemented::refusal(stmt))
                .flatten()
            {
                Some(diagnostic) => Err(diagnostic),
                None => lower_top_level_item(stmt, &mut state, resolved, position),
            },
        };
        match outcome {
            Ok(()) => {
                // P5: an item that binds a poisoned name and lowers
                // un-poisons it. `retain`, never `position` + `remove`, so a
                // duplicate could never survive even if one were inserted.
                for name in poisonable_names(stmt) {
                    poisoned.retain(|poisoned_name| poisoned_name != name);
                }
            }
            Err(diagnostic) => {
                failed_items.push(index);
                // P2: a cascade-shaped error naming a poisoned binding is a
                // consequence of the earlier skip, not a new gap -- skip
                // this item silently (P4). Anything else is reported.
                let is_cascade = cascade_name(&diagnostic)
                    .is_some_and(|name| poisoned.iter().any(|poisoned_name| poisoned_name == name));
                if !is_cascade {
                    diagnostics.push(diagnostic);
                }
                // P1: whatever the reason, the item bound nothing, so the
                // names it would have bound are now poisoned (transitively
                // for a cascade skip). Insert only if absent.
                for name in poisonable_names(stmt) {
                    if !poisoned.iter().any(|poisoned_name| poisoned_name == name) {
                        poisoned.push(name.to_string());
                    }
                }
            }
        }
        // #944 (D-233): the second per-item collection source. Scanned after
        // the item's own outcome (Ok or Err alike) so the item's diagnostic
        // precedes its enum-call diagnostics and the whole list stays in
        // loop order (D-217 rule 3, never re-sorted). The name set is the
        // syntactic pre-collection plus every enum class known to `state`
        // at this point (an enum a project import pulled in, keyed on the
        // `is_enum` provenance flag, never on `enum_members` emptiness),
        // minus the poisoned names: a call to an enum class that itself
        // failed to lower is a cascade of that skip (D-219, P2); minus the
        // names another module-level statement binds too (collision,
        // limit (vii)).
        // Borrowed names, and no walk at all when the set is empty (the
        // common module, which defines and imports no enum class): the scan
        // is a full AST walk of the item plus one of each `def` body, and
        // paying it unconditionally cost the `pycc check` frontend bench
        // ~7% (PR #971's `frontend-perf-gate`).
        let mut enum_class_names: Vec<&str> = syntactic_enum_classes
            .iter()
            .map(String::as_str)
            .chain(
                state
                    .class_defs
                    .iter()
                    .filter(|(_, class_def)| class_def.is_enum)
                    .map(|(name, _)| name.as_str()),
            )
            .filter(|name| !poisoned.iter().any(|poisoned_name| poisoned_name == name))
            .collect();
        if enum_class_names.is_empty() {
            continue;
        }
        let rebound_names = rebound_names
            .get_or_insert_with(|| class::enum_call::module_rebound_names(&module.body));
        enum_class_names.retain(|name| !rebound_names.iter().any(|rebound| rebound == name));
        if !enum_class_names.is_empty() {
            // `state.imports` at this point is exactly what `lower_stmt`
            // folded this item's `TYPE_CHECKING` guards against, so the scan
            // skips the same dead bodies the lowering did.
            let module_frame = module_frame.get_or_insert_with(|| {
                class::enum_call::module_bindings(&module.body, &state.imports[..pre_loop_imports])
            });
            diagnostics.extend(class::enum_call::reject_enum_class_calls(
                stmt,
                module_frame,
                &enum_class_names,
                &state.imports,
            ));
        }
    }
    let loop_end = diagnostics.len();
    // Part 1 of #1026, PR 1c of #1080: the whole item list exists only
    // here, so this is the first point at which both orders of a shadowed
    // foreign import -- a `def`/assignment above it and one below it -- are
    // visible at once. Appended to the same per-item diagnostic list so a
    // module with an earlier failure still reports that one first.
    diagnostics.extend(crate::import::reject_shadowed_foreign_imports(
        &state.imports,
        &state.definition_spans,
        &state.fallback_groups,
    ));
    // #1380: a buffer-carrier from-import binds a hidden name, so every
    // other use of its spelling -- above or below the import -- is refused
    // here, where the whole module is visible at once, then spliced in
    // after the diagnostics of the item each refusal sits in, so the list
    // keeps per-item source order.
    let carrier_misuse =
        crate::import::reject_carrier_misuse(&module.body, resolved, &failed_items);
    let mut diagnostics = crate::import::splice_by_item(
        &module.body,
        &item_starts,
        loop_end,
        diagnostics,
        carrier_misuse,
    );
    // #1244: the module-level `del` late-binding rule, after the per-item
    // loop so an earlier per-item failure still reports first.
    let deleted_top_level =
        stmt::del::check_module_deletions(module).unwrap_or_else(|diagnostic| {
            diagnostics.push(diagnostic);
            Vec::new()
        });
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    let ModuleState {
        aliases,
        imports,
        class_defs,
        class_asts: _,
        mut class_slots,
        items,
        imported_class_indices,
        imported_alias_indices,
        definition_spans,
        fallback_groups: _,
        signatures,
        staticmethod_rebound: _,
        type_vars: _,
    } = state;
    // The imported copies were pushed after the synthetic set, so
    // stripping them leaves the synthetic entries still at the front.
    let mut class_defs = strip_imported(class_defs, &imported_class_indices);
    let aliases = strip_imported(aliases, &imported_alias_indices);
    class_defs.rotate_left(synthetic_class_count);
    class_slots.retain(|(name, _)| class_defs.iter().any(|(class, _)| class == name));
    Ok(LoweredModule {
        hir: HirModule {
            items,
            type_aliases: aliases,
            imports,
            class_defs,
            seeded_builtin_exception_classes,
        },
        shadowed_builtin_exception_name,
        mentions_dunder_name,
        definition_spans,
        container_method_names,
        deleted_top_level,
        mentioned_names: None,
        class_slots,
        // Read after the item loop (#1425): the final state, including the
        // module's own top-level and block foreign imports.
        object_receivers: signatures.object_receivers(),
    })
}

/// Lowers one top-level statement into `state`, in exactly the order the
/// pre-#867 loop body did: type alias, legacy type alias, import, class,
/// then function or plain statement, each with its own reverse-direction
/// name-collision checks. On `Err` nothing was recorded into `state`, which
/// is what lets `lower_module` skip the item as a unit.
fn lower_top_level_item<'a>(
    stmt: &'a Stmt,
    state: &mut ModuleState<'a>,
    resolved: &ResolvedImports<'_>,
    position: FuturePosition,
) -> Result<(), Diagnostic> {
    // #380 (PR-20): build the projected class slice `annotation_to_ty`
    // uses to resolve cross-class annotations; #693 (PEP 560) added the
    // per-class `__class_getitem__` return type it carries. #611's
    // per-class subscriptability flag rode here too until #1130 deleted it
    // along with the annotation-position gate it fed. Recomputed per item
    // so a later item sees every earlier class.
    let class_name_defs = class::class_annotation_infos(&state.class_defs, &state.items);
    if let Some((name, ty)) = lower_type_alias_stmt(stmt, &state.aliases, &class_name_defs)? {
        // D-068 review finding on #385, second round: the class-vs-alias
        // check below (at the `Stmt::ClassDef` arm) only ever catches a
        // class defined *after* a same-named alias -- without this
        // check, `class Foo: ...` followed by `type Foo = int` would
        // silently establish a second, alias-shaped `Foo` binding with
        // no diagnostic, the exact failure mode this finding exists to
        // close, just in the untreated direction.
        if state
            .class_defs
            .iter()
            .any(|(class_name, _)| *class_name == name)
        {
            return Err(unsupported(
                format!(
                    "type alias `{name}` collides with a class of the same name \
                     already defined in this module"
                ),
                pycc_ast::stmt_range(stmt),
            ));
        }
        state
            .definition_spans
            .push((name.clone(), statement_span(stmt)));
        state.aliases.push((name, ty));
        return Ok(());
    }
    if let Some((name, ty)) =
        lower_legacy_type_alias_ann_assign(stmt, &state.aliases, &class_name_defs)?
    {
        // Same reverse-direction check as the `type X = ...` arm above,
        // for the legacy `X: TypeAlias = <expr>` spelling.
        if state
            .class_defs
            .iter()
            .any(|(class_name, _)| *class_name == name)
        {
            return Err(unsupported(
                format!(
                    "type alias `{name}` collides with a class of the same name \
                     already defined in this module"
                ),
                pycc_ast::stmt_range(stmt),
            ));
        }
        state
            .definition_spans
            .push((name.clone(), statement_span(stmt)));
        state.aliases.push((name, ty));
        return Ok(());
    }
    if type_var::lower_type_var_decl(stmt, state)? {
        return Ok(());
    }
    // Part 1 of #1026: `state.items.len()` at this exact point is the
    // number of `HirItem`s the preceding module statements produced, which
    // is the interleaving position an `ImportBinding::Foreign` records.
    if let Some(mut lowered) = lower_import_stmt(
        stmt,
        resolved,
        position,
        ForeignImportSite::Item(state.items.len()),
    )? {
        // Same reverse-direction check as the two type-alias arms above,
        // for `import ...`/`from ... import ...` (a single statement can
        // bind more than one local name, e.g. `from math import sqrt,
        // pi`, so every bound name is checked, not just the first). A
        // class this module imported earlier is exempt: `from a import
        // Point` twice binds the same definition twice, not a collision.
        if let Some(index) = colliding_class_import(state, &lowered.bindings) {
            return Err(class_collision(
                import_local_name(&lowered.bindings[index]),
                pycc_ast::stmt_range(stmt),
            ));
        }
        // #898: bring an imported class (with its MRO) and an imported
        // alias into this module's tables so later annotations and bases
        // resolve them; a copy already present (an ancestor shared with an
        // earlier import, or a synthetic exception class this module seeded
        // itself) is not duplicated.
        //
        // The two `continue` guards below have no observable effect on
        // Part 1's output, and no test discriminates them: every copy they
        // would skip is also recorded in `imported_class_indices` /
        // `imported_alias_indices` and removed again by `strip_imported`
        // before anything downstream sees the module, and the name lookups
        // in between take the first match over byte-identical entries.
        // They are kept because they hold the one-name-one-entry invariant
        // that the collision checks just above and the `HashMap`-collected
        // class tables downstream (`pycc_types::Environment::classes`,
        // `pycc_mir`'s own `classes` map) are written against. Part 2
        // (#899, per-module namespaces) is where stripping stops being
        // universal and the guards become load-bearing.
        for entry in lowered.classes {
            if state.class_defs.iter().any(|(name, _)| *name == entry.0) {
                continue;
            }
            state.imported_class_indices.push(state.class_defs.len());
            state.class_defs.push(entry);
        }
        for row in lowered.class_slots {
            if !state.class_slots.iter().any(|(name, _)| *name == row.0) {
                state.class_slots.push(row);
            }
        }
        for entry in lowered.aliases {
            if state.aliases.iter().any(|(name, _)| *name == entry.0) {
                continue;
            }
            state.imported_alias_indices.push(state.aliases.len());
            state.aliases.push(entry);
        }
        // Part 1 of #1367: a name a foreign import binds -- through any of
        // its channels, `import X`, #1278's `from m import C`, #1366's
        // relative and #1138's dotted from-import -- is spellable in every
        // annotation position the alias table serves, as the opaque
        // `Ty::Object` (`docs/TYPE_SYSTEM.md`, the `object` row). CPython
        // never checks an annotation, so no run-time check follows. The
        // entry is recorded as imported, so `strip_imported` keeps it out of
        // `HirModule::type_aliases`: the name neither re-exports nor leaks,
        // while a D-135 alias built from it is an ordinary alias of `object`.
        for binding in &lowered.bindings {
            if let ImportBinding::Foreign { local_name, .. } = binding
                && !state.aliases.iter().any(|(name, _)| name == local_name)
            {
                state.imported_alias_indices.push(state.aliases.len());
                state.aliases.push((local_name.clone(), Ty::Object));
            }
        }
        state.imports.append(&mut lowered.bindings);
        return Ok(());
    }
    if let Stmt::ClassDef(def) = stmt {
        let (class_def, mut method_items) = class::lower_class(
            def,
            &state.aliases,
            &state.class_defs,
            &state.items,
            &state.class_asts,
            &state.imports,
            &state.type_vars,
            &state.signatures,
            state.staticmethod_rebound,
        )?;
        // D-154 Part 1's own post-merge review finding: two module-level
        // classes sharing a name would each lower their own `__init__`
        // (and any other same-named method) to the identical mangled
        // `<Name>.<method>` function name, silently colliding in
        // `HirModule::items`/`class_defs`'s `HashMap`-collected class
        // table downstream (`pycc_types::Environment::classes`,
        // `pycc_mir`'s own `classes` map) rather than producing a clean
        // diagnostic -- reject it here, at the same point `lower_class`'s
        // own duplicate-method check (`crates/pycc_hir/src/class.rs`)
        // fires for the identical shape one level down.
        // #898: a class copied in by a project import is not a definition
        // of this module, so a same-named `class` here is reported by the
        // import-collision check below, not as a duplicate definition.
        if state
            .class_defs
            .iter()
            .enumerate()
            .any(|(index, (name, _))| {
                name == &class_def.name && !state.imported_class_indices.contains(&index)
            })
        {
            return Err(unsupported(
                format!(
                    "class `{}` is defined more than once in this module",
                    class_def.name
                ),
                def.range,
            ));
        }
        // D-068 review finding on #385: a class name colliding with an
        // already-defined top-level function, type alias, or import
        // name produced no diagnostic and silently, permanently
        // shadowed the earlier binding -- `pycc_types::Environment`
        // checks `env.lookup_class(callee)` before the ordinary
        // function lookup at every call site (`crates/pycc_types/src/
        // lib.rs`), on the (until now unenforced) assumption that a
        // class name can never collide with a real function name in
        // this compiler's flat, single-namespace model. Enforce that
        // assumption here, at the same point the class-vs-class check
        // above already fires, rather than leaving it merely asserted
        // in a comment one crate over. Only a top-level function name
        // is checked against `items` (a method's own mangled
        // `<ClassName>.<method>` name can never collide with a bare
        // class name -- a real Python `NAME` token can never contain a
        // `.`, `pycc_hir::class`'s own doc comment).
        if state
            .items
            .iter()
            .any(|item| matches!(item, HirItem::Function { name, .. } if *name == class_def.name))
        {
            return Err(unsupported(
                format!(
                    "class `{}` collides with a function of the same name already \
                     defined in this module",
                    class_def.name
                ),
                def.range,
            ));
        }
        // Part 1 of #1367: the alias entry a foreign import records for its
        // bound name is not a type alias the source wrote, so a class of the
        // same name keeps the import-collision message below.
        let foreign_bound = |alias: &str| {
            state.imports.iter().any(|binding| {
                matches!(binding, ImportBinding::Foreign { local_name, .. } if local_name == alias)
            })
        };
        if state
            .aliases
            .iter()
            .any(|(name, _)| name == &class_def.name && !foreign_bound(name))
        {
            return Err(unsupported(
                format!(
                    "class `{}` collides with a type alias of the same name already \
                     defined in this module",
                    class_def.name
                ),
                def.range,
            ));
        }
        if state
            .imports
            .iter()
            .any(|binding| import_local_name(binding) == class_def.name)
        {
            return Err(unsupported(
                format!(
                    "class `{}` collides with an import of the same name already \
                     defined in this module",
                    class_def.name
                ),
                def.range,
            ));
        }
        // #974 (round 6, D-068 re-review): a class statement that follows a
        // top-level *value* binding of the same name is rejected here, for
        // the same reason the four checks above reject the other collisions
        // -- `HirItem` has no `ClassDef` variant, so a class statement never
        // appears in `items` and the type checker's sequential top-level
        // pass can never re-bind the name back to the class after the value
        // assignment. Every later read of the bare name therefore resolves
        // through the stale value binding, which is a silently wrong answer
        // rather than a diagnostic (CPython evaluates the class statement
        // and rebinds the name). This walk is the one place that still sees
        // the two in source order, so the rejection belongs here.
        //
        // The check is deliberately one-directional: only bindings from
        // statements *already* lowered are considered, so the reverse order
        // (`class A: ...` then `A = [1]`) stays accepted -- there the value
        // assignment is itself a top-level statement the checker's
        // sequential pass does observe, and reading `A` as a value after it
        // is correct.
        if state.items.iter().any(|item| {
            matches!(item, HirItem::TopLevelStmt(stmt)
                if killed_names(std::slice::from_ref(stmt)).contains(&class_def.name))
        }) {
            return Err(unsupported(
                format!(
                    "class `{}` collides with a value of the same name already \
                     bound at module scope earlier in this module -- pycc has no \
                     representation for a class statement rebinding a name that \
                     already holds a value, so a later read of `{}` would resolve \
                     through the stale value binding",
                    class_def.name, class_def.name
                ),
                def.range,
            ));
        }
        let own_slots = class::slots::check_class(
            def,
            &class_def,
            &state.class_defs,
            &state.class_slots,
            &state
                .aliases
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
        )?;
        state.class_slots.push((class_def.name.clone(), own_slots));
        state
            .definition_spans
            .push((class_def.name.clone(), statement_span(stmt)));
        state.class_asts.push((class_def.name.clone(), def));
        state.class_defs.push((class_def.name.clone(), class_def));
        state.items.append(&mut method_items);
        return Ok(());
    }
    if let Stmt::FunctionDef(def) = stmt
        && state
            .class_defs
            .iter()
            .any(|(name, _)| name == def.name.as_str())
    {
        // The reverse direction of the check above: a top-level
        // function defined *after* a same-named class must be rejected
        // too, not only a class defined after a same-named function.
        return Err(unsupported(
            format!(
                "function `{}` collides with a class of the same name already \
                 defined in this module",
                def.name
            ),
            def.range,
        ));
    }
    let span = statement_span(stmt);
    if let Stmt::FunctionDef(def) = stmt {
        let item = lower_function(
            def,
            &state.aliases,
            &class_name_defs,
            &state.imports,
            &state.signatures,
        )?;
        if let HirItem::Function { name, .. } = &item {
            state.definition_spans.push((name.clone(), span));
        }
        state.items.push(item);
        return Ok(());
    }
    // #1291: the foreign imports nested in a module-level `if`/`try` go
    // into the import table before the block is lowered, because
    // `lower_stmt` reads them there to produce `HirStmt::ForeignImport`.
    // A block that then fails to lower leaves none of them behind, so an
    // import that never runs is never a lock root, a policy (I0402) or a
    // native-build (I0403) finding.
    let block_imports = crate::import::lower_block_imports(stmt, resolved, &state.imports);
    // The same class-name collision the top-level arm refuses above: a
    // nested `import numpy as ValueError` would otherwise bind a name the
    // module (or its seeded builtin exception classes) already defines.
    if let Some(index) = colliding_class_import(state, &block_imports.bindings) {
        let span = block_imports.spans[index];
        return Err(class_collision(
            import_local_name(&block_imports.bindings[index]),
            span.start..span.end,
        ));
    }
    let imports_before_block = state.imports.len();
    state.imports.extend(block_imports.bindings.iter().cloned());
    // #1213: a chained assignment expands into several statements, all
    // lowered before any is recorded, so an `Err` still records nothing.
    let lowered = stmt::lower_stmt_expanded(
        stmt,
        &state.aliases,
        false,
        false,
        false,
        // #795 (PEP 654): module top level is `Outside` by definition.
        stmt::ExceptStarCtx::Outside,
        None,
        None,
        &class_name_defs,
        &state.imports,
        &state.signatures,
    )
    .map_err(|error| {
        state.imports.truncate(imports_before_block);
        // A nested import that failed to lower reports what the same line
        // reports at top level, when it is the block's first failure.
        block_imports.substitute(error)
    })?;
    // A synthesized name -- a chained-assignment temporary (`0chain_<offset>`,
    // #1213), a tuple-unpacking temporary (`0unpack_<offset>`, Part 1 of
    // #891) or a comprehension loop variable (`0comp_<offset>_<name>`,
    // D-117, #1237) -- is not a definition the source wrote, so it never
    // takes part in `program::link`'s cross-module collision check -- the
    // same reason the `__name__` seed is not recorded (see `lower_module`).
    // `killed_names` reaches into nested bodies, so one inside a
    // module-level `for` or `if` is filtered the same way.
    // #1485: an `except ImportError` fallback rebinding is no definition.
    let groups = crate::import::fallback_groups(&lowered, span).inspect_err(|_| {
        state.imports.truncate(imports_before_block);
    })?;
    for name in killed_names(&lowered) {
        if !is_synthesized_name(&name) && !groups.iter().any(|group| group.name == name) {
            state.definition_spans.push((name, span));
        }
    }
    state.fallback_groups.extend(groups);
    state
        .items
        .extend(lowered.into_iter().map(HirItem::TopLevelStmt));
    Ok(())
}

/// Whether `name` was synthesized by lowering rather than written in the
/// source. Every synthesized name -- #1213's `0chain_<offset>`, #891's
/// `0unpack_<offset>` and D-117's `0comp_<offset>_<name>` -- starts with an ASCII digit, which no Python
/// identifier can, so the test cannot match a source name.
pub(crate) fn is_synthesized_name(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

fn statement_span(stmt: &Stmt) -> Span {
    let range = pycc_ast::stmt_range(stmt);
    Span::new(range.start, range.end)
}

#[cfg(test)]
mod tests;

/// The index of the first of `bindings` whose local name is a class this module defines
/// (or seeded, like the builtin exception classes) rather than imported --
/// the reverse-direction class-name collision both the top-level import arm
/// and a module-level block's nested foreign imports (#1291) refuse. A
/// class this module imported earlier is exempt: `from a import Point`
/// twice binds the same definition twice, not a collision.
fn colliding_class_import(state: &ModuleState<'_>, bindings: &[ImportBinding]) -> Option<usize> {
    bindings.iter().position(|binding| {
        let local_name = import_local_name(binding);
        state
            .class_defs
            .iter()
            .enumerate()
            .any(|(index, (class_name, _))| {
                class_name == local_name && !state.imported_class_indices.contains(&index)
            })
    })
}

/// The `C0001` for [`colliding_class_import`]'s finding.
fn class_collision(colliding: &str, range: std::ops::Range<u32>) -> Diagnostic {
    unsupported(
        format!(
            "import `{colliding}` collides with a class of the same name \
             already defined in this module"
        ),
        range,
    )
}
