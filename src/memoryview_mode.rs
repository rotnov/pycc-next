//! The native-mode gate on a `memoryview` annotation (Part 1 of #1027).
//!
//! A `memoryview` parameter is a borrowed view of storage a CPython
//! interpreter owns: the value only exists because a generated
//! `pycc build --ext` wrapper acquired a `Py_buffer` from a real
//! `memoryview` object and released it again when the call returned
//! (`src/ext_build.rs`'s wrapper generator owns that discipline). A plain
//! `pycc build` produces a standalone native executable with no interpreter
//! at all, so there is nothing to acquire a buffer *from*, and the program
//! is refused here rather than compiled into a signature nothing could ever
//! call.
//!
//! This is a sibling of `src/foreign_import.rs`, and for the same reason:
//! `crates/pycc_hir` and `crates/pycc_types` have no artifact-mode
//! awareness at all, so a refusal that depends on the mode cannot live in
//! either. `src/frontend.rs`'s `resolve_frontend_native` is the one seam
//! that knows.
//!
//! It refuses no miscompile, and that is recorded rather than left for a
//! later reader to rediscover as dead weight: Part 1 adds no expression
//! that *produces* a `memoryview` value (the constructor is still refused),
//! so no call site inside a native program could type-check against such a
//! signature in the first place. The gate exists to make the refusal
//! legible -- a named code, a sentence naming `--ext`, and a test pinning
//! both -- instead of leaving a `memoryview` annotation to fail later with
//! a message about something else.

use pycc_diag::{Diagnostic, Severity};
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, ProtocolMember, Ty};
use std::collections::{HashMap, HashSet};

/// The `I04xx` code this gate emits. The family is the CPython interop
/// boundary (`docs/DIAGNOSTICS.md`): `I0403` is its nearest neighbour --
/// a CPython *import* in native mode -- and this is a CPython *buffer* in
/// native mode.
pub(crate) const NATIVE_MEMORYVIEW_CODE: &str = "I0405";

/// One [`NATIVE_MEMORYVIEW_CODE`] per function whose signature names
/// `memoryview`, paired with that function's index in `hir.items`, or
/// `Ok(())` when the program names it nowhere.
///
/// Shaped like `src/ext_build.rs`'s `collect_exports` rather than like
/// `src/foreign_import.rs`'s import walk: a `memoryview` annotation is not
/// an entry in the flat import table, it is a `Ty` on an `HirItem::Function`
/// in `hir.items`. Every offending function is reported, not only the
/// first, so one build names the whole list.
///
/// Span-less, exactly like `src/ext_build.rs`'s `capability_gap`:
/// `HirItem::Function` carries no source range -- that is the point of
/// `pycc_hir`'s lowered form -- and `pycc_diag::render_human` renders a
/// span-less diagnostic as plain `error[I0405]: <message>`. The item index
/// still names the owning *file*, which is what `src/frontend.rs` joins it
/// against.
///
/// Private functions are gated too, unlike the `--ext` export set: the
/// refusal is about the artifact having no interpreter, which a `_`-prefixed
/// name does not change.
///
/// `src/frontend.rs`'s `resolve_frontend_native` reports these gaps *before*
/// the type check, unlike the foreign-import gate beside it: otherwise a body
/// that reads its own `memoryview` parameter fails
/// `crates/pycc_types`'s `reject_memoryview_read` first and the signature
/// never gets the code it is documented to get. That call site owns the
/// reasoning.
///
/// A *signature* is the whole of this gate's job. `pycc_hir`'s
/// `annotation_to_ty` also parses a bare `x: memoryview` declaration, which
/// this walk never reaches -- it visits `params` and `return_ty`, not
/// statement bodies -- and which no artifact mode admits at all;
/// `crates/pycc_types`'s `reject_memoryview_declaration` refuses that
/// position with `C0001` in both modes.
pub(crate) fn refuse_in_native_mode(hir: &HirModule) -> Result<(), Vec<(usize, Diagnostic)>> {
    let gaps: Vec<(usize, Diagnostic)> = hir
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let HirItem::Function {
                name,
                params,
                return_ty,
                ..
            } = item
            else {
                return None;
            };
            let position = offending_position(params, return_ty)?;
            Some((index, gap(name, &position)))
        })
        .collect();
    if gaps.is_empty() {
        return Ok(());
    }
    Err(gaps)
}

/// The protocol-method half of [`refuse_in_native_mode`]: one
/// [`NATIVE_MEMORYVIEW_CODE`] per protocol method whose *parameter* list
/// names `memoryview`, paired with that class's index in `hir.class_defs`.
///
/// It needs its own walk because a protocol method is never lowered to an
/// `HirItem::Function` -- `crates/pycc_hir/src/class/protocol.rs` records it
/// as a `ProtocolMember::Method` and says so in its header -- so the
/// `hir.items` walk above cannot see the signature at all, and a
/// `class P(Protocol): def total(self, v: memoryview) -> int: ...` built
/// natively escaped the documented `I0405` entirely.
///
/// Only the parameter position is checked here, and that split is the whole
/// design: a `memoryview` *return* type is unsatisfiable in every artifact
/// mode, so `protocol.rs` refuses it at the declaration with `C0001` for
/// both modes at once, while a `memoryview` *parameter* is mode-dependent --
/// under `pycc build --ext` a class really can satisfy it, because an
/// exported wrapper acquires the buffer and passes the view inward (measured:
/// a `def total(self, v: memoryview) -> int` method builds under `--ext`
/// today). Native mode is exactly where it becomes impossible, which is this
/// gate's own subject.
///
/// Keyed by class index rather than item index, so the caller resolves it
/// through `ProgramSources::owner_of_class` instead of `owner_of_item`:
/// `class_defs` is its own concatenated table with its own per-file bounds,
/// and a protocol class defined in an imported module is contributed by that
/// module, not by the entry file.
///
/// An *inherited* member is skipped. `lower_protocol_class` copies a base
/// protocol's members into the derived class's own `protocol_members`, so a
/// `class Q(P, Protocol)` carries `P`'s offending method verbatim; reporting
/// it again would name `Q` for a signature `P` declares. The declaring class
/// is the one that gets the diagnostic, exactly as for the return position.
///
/// "Inherited" means the *whole member* matches an ancestor's, not merely
/// its name. A derived protocol may override an ancestor's method with a
/// different signature -- `lower_protocol_class` replaces the copied entry
/// when the body redeclares the name -- so `class P(Protocol): def f(self,
/// x: int)` followed by `class Q(P): def f(self, v: memoryview)` leaves `Q`
/// as the declaring class of an offending signature that `P` never had.
/// Comparing by name alone would skip it and let the native build through.
///
/// Span-less and parameter-name-less, both for the same reason as
/// [`gap`]: `ProtocolMember::Method` carries `param_tys: Vec<Ty>` and no
/// names at all, so the position is named by its 1-based index with `self`
/// already stripped -- the shape `protocol.rs` records.
pub(crate) fn refuse_protocol_methods_in_native_mode(
    hir: &HirModule,
) -> Result<(), Vec<(usize, Diagnostic)>> {
    let by_name: HashMap<&str, &Vec<ProtocolMember>> = hir
        .class_defs
        .iter()
        .map(|(name, def)| (name.as_str(), &def.protocol_members))
        .collect();
    let mut gaps: Vec<(usize, Diagnostic)> = Vec::new();
    for (index, (class_name, def)) in hir.class_defs.iter().enumerate() {
        for member in &def.protocol_members {
            let ProtocolMember::Method {
                name, param_tys, ..
            } = member
            else {
                continue;
            };
            let Some(position) = param_tys.iter().position(|ty| *ty == Ty::MemoryView) else {
                continue;
            };
            if def.mro.iter().any(|ancestor| {
                ancestor != class_name && declares_member(&by_name, ancestor, member)
            }) {
                continue;
            }
            gaps.push((
                index,
                gap(
                    &format!("{class_name}.{name}"),
                    &format!("parameter {} `memoryview`", position + 1),
                ),
            ));
        }
    }
    if gaps.is_empty() {
        return Ok(());
    }
    Err(gaps)
}

/// Whether `class_name` names a protocol class that declares `member`
/// itself -- the same name *and* the same signature, so that the copy in
/// the derived class really is the ancestor's and not an override of it.
/// A name the class table does not hold (a builtin base, `Protocol`
/// itself) declares nothing.
fn declares_member(
    by_name: &HashMap<&str, &Vec<ProtocolMember>>,
    class_name: &str,
    member: &ProtocolMember,
) -> bool {
    by_name
        .get(class_name)
        .is_some_and(|members| members.iter().any(|m| m == member))
}

/// Names the first part of a signature that mentions the buffer type, in
/// its canonical spelling, or `None` when it mentions it nowhere.
///
/// The rendered spelling is `memoryview` whichever of the three source
/// spellings the user wrote (#1129 admits `ndarray` as the second, #1134
/// `NDArray` as the third), for the reason `Ty::MemoryView`'s own
/// documentation gives: one type, one canonical name, exactly as for an
/// alias of it.
///
/// Both positions are checked, and only `Ty::MemoryView` itself matches,
/// with no recursion into a container's elements. A parameterized container
/// annotation does lower its elements through the same `annotation_to_ty`
/// recursion, so `list[memoryview]` -- and, since #1129, `list[ndarray]` --
/// really does produce `Ty::List(Ty::MemoryView)`. What keeps that type out
/// of a lowered function's `params` and `return_ty` is the capability gate
/// one step later: `pycc_hir`'s `check_container_ty` admits only `Ty::Int`
/// as a list element and rejects every other one with `T0034`. So the
/// nested shape is unreachable here by a type-directed refusal, not by a
/// name that fails to resolve.
fn offending_position(params: &[(String, Ty)], return_ty: &Ty) -> Option<String> {
    if let Some((name, _)) = params.iter().find(|(_, ty)| *ty == Ty::MemoryView) {
        return Some(format!("parameter `{name}: memoryview`"));
    }
    if *return_ty == Ty::MemoryView {
        return Some("return type `-> memoryview`".to_string());
    }
    None
}

/// The `--ext` counterpart: one `C0001` per function whose *return* type is
/// `memoryview`, or `Ok(())` when none is.
///
/// `src/ext_build.rs`'s `collect_exports` already refuses that return type
/// on a *public* function, and on a public `@staticmethod` or
/// `@classmethod` of a public non-exception class, with `C0003` and the
/// export-boundary wording. This closes the rest of the program: a private
/// `def _make() -> memoryview`, an instance method, a property, a method of
/// a private or exception class, or a monomorphized specialization is
/// skipped by that walk entirely, so nothing refused the signature and
/// lowering the call's result reached `pycc_codegen`'s "a
/// `memoryview`-typed call result is not supported yet" panic -- a compiler
/// crash on valid Python, not a diagnostic. The `C0003` message's own
/// advice ("rename it to `_name` to keep it out of the export set") pointed
/// straight at that crash.
///
/// **Superseded by Part 2b of #1142 (#1164) for the `C0003` half.** A
/// buffer return type is carriable now, so no function reaches
/// `collect_exports`' gap set on account of it and the `C0003` takeover
/// described above no longer happens: a public `memoryview`-returning
/// `@staticmethod` or `@classmethod` is admitted by the export boundary and
/// refused here, with this function's own `C0001`, for the reason the
/// exemption paragraph below gives. The suppression note still describes
/// `collect_exports` correctly for every *other* unexportable signature.
///
/// `C0001` rather than `C0003`: `docs/DIAGNOSTICS.md` defines `C0003` as a
/// *public* function's signature failing to cross the boundary, which a
/// private one is not. This is the same versioned capability gap
/// `crates/pycc_types`'s `reject_memoryview_declaration` and
/// `reject_memoryview_read` report. Its *ground* has changed since #1027:
/// a buffer-producing expression now exists (`a = ndarray(n)`, Part 2a of
/// #1142), so the gap is no longer "nothing could produce one" but "only
/// one position can hand one out". The message is worded like those two
/// siblings, and for the same reason: it names the type as "a buffer"
/// rather than in any of its three spellings (#1134 added `NDArray`), so a
/// user who wrote `ndarray` or `NDArray` is not told about a `memoryview`
/// they never mentioned. The
/// canonical-spelling rendering `offending_position` does is the *native*
/// path's, whose `I0405` quotes a whole signature position back; this
/// message quotes no annotation text at all.
///
/// Only the return position is checked. A `memoryview` *parameter* on a
/// private function carries no value into codegen either way: for the same
/// reason there is nothing to return, there is nothing to pass, so no call
/// to such a function can be written at all -- whether its body reads the
/// parameter with the `b[i]` load Part 2 of #1027 admits or is refused a
/// `C0001` for reading it any other way. And native mode is not this
/// function's concern --
/// [`refuse_in_native_mode`] refuses both positions there, private
/// functions included.
///
/// Called after `collect_exports`, and handed its result: Part 2b of #1142
/// (#1164) made a buffer return type *carriable*, so the export set is no
/// longer a set this walk can assume was already refused.
///
/// The exemption is exactly "is exported" (#1174). It was once narrower --
/// **exported and module-level**, a dot-free name -- because
/// `pycc_codegen`'s `call_result.rs` holds a `Ty::MemoryView` panic that
/// #1164's completion criteria require to stay unreachable, and `pycc_types`
/// intercepted a buffer-returning *call*
/// (`crate::buffer::buffer_returning_call_unsupported`) only at
/// `HirExpr::Call`, the sole route to a module-level `def`. A method's
/// return type resolves instead through `class::method_call::
/// resolve_method_call`, `class::static_call::
/// resolve_static_or_class_method_call` and `class::super_call::
/// resolve_super_method_call` from `HirExpr::MethodCall`, so `g.make()`
/// inside the artifact would have reached the panic. #1174 extended the
/// interception to all four of those resolver exits
/// (`crate::buffer::refuse_buffer_returning_method`), which is what lets the
/// declaration be admitted here: the panic stays unreachable because every
/// *intra-artifact* route to the return value is refused, not because the
/// method cannot be written.
///
/// Part 1 of #1175 (#1178) adds a *second provenance* to the value such an
/// export may return -- the bare name a `memoryview` **parameter** binds,
/// handed back over a second, independent buffer export the wrapper takes on
/// the host's own argument object -- and Part 2 (#1179) adds a *third*, a
/// step-free sub-range of that parameter. Neither widens this predicate,
/// which is export-set membership and asks nothing about provenance. The
/// D-141 argument above is therefore unchanged in structure but now covers
/// three provenances: every *intra-artifact* route to a buffer-returning
/// callee is still refused, whichever provenance its body
/// returns, because each of those interceptions keys on the callee's
/// declared return type and never on what its body does. That is what keeps
/// `call_result.rs`'s `Ty::MemoryView` panic unreachable for the shapes
/// #1175 newly admits -- a callee whose whole body is `return b` could not
/// type-check at all before it, so the routes are re-asserted against one in
/// `tests/issue_1175_caller_owned_buffer_return.rs` rather than assumed to
/// carry over, and `tests/issue_1179_buffer_slice_return.rs` re-asserts the
/// one route the sub-range provenance adds.
///
/// What still refuses a declaration is therefore exactly non-membership in
/// the export set, and `collect_exports` owns every reason for that: a
/// private name (leading underscore, on the method or on its class), a
/// method of an exception class, an unreachable instance method, a
/// `@property` getter, and a private module-level `def`.
pub(crate) fn refuse_in_ext_mode(
    hir: &HirModule,
    exports: &[crate::ext_build::ExtExport],
) -> Result<(), Vec<Diagnostic>> {
    let gaps: Vec<Diagnostic> = hir
        .items
        .iter()
        .filter_map(|item| {
            let HirItem::Function {
                name, return_ty, ..
            } = item
            else {
                return None;
            };
            if *return_ty != Ty::MemoryView {
                return None;
            }
            // #1174: the name is compared as-is, mangled dots included --
            // a method arrives here as `Grid.make` or `Grid.make.static`,
            // which is exactly the spelling `collect_exports` records in
            // `ExtExport::name`.
            let exported = exports.iter().any(|export| export.name == *name);
            (!exported).then(|| ext_return_gap(name))
        })
        .collect();
    if gaps.is_empty() {
        return Ok(());
    }
    Err(gaps)
}

fn ext_return_gap(name: &str) -> Diagnostic {
    // The *source-level* spelling, for the reason `capability_gap` states in
    // full: a method arrives mangled, and Part 2b of #1142 (#1164) made this
    // message the one a public `@staticmethod` gets -- so left alone it would
    // name `Grid.make.static`, which the user never wrote. It was `C0003`,
    // which already rendered the source-level name, that reported that case
    // before.
    let name = crate::ext_build::source_level_name(name);
    Diagnostic {
        code: "C0001",
        severity: Severity::Error,
        message: format!(
            "`{name}`'s return type is a buffer, which is valid Python but not implemented \
             yet; Part 2b of #1142 (#1164) hands a buffer back to the CPython host from a \
             `pycc build --ext` *export* only, and #1174 admits a public method of a \
             public class as one -- but no unexported function may return one, so move \
             the buffer-producing code into a public module-level `def` or a public \
             method of a public class"
        ),
        span: None,
        label: None,
        help: None,
    }
}

/// One [`NATIVE_MEMORYVIEW_CODE`] per function body that binds
/// artifact-owned buffer storage, paired with that function's index in
/// `hir.items`, or `Ok(())` when the program allocates none (Part 2a of
/// #1142, issue #1165).
///
/// The third native-mode buffer gate, and the first that reads *bodies*
/// rather than signatures. It exists because #1165 gave `Ty::MemoryView` a
/// second source that the two signature walks above structurally cannot
/// see: `a = ndarray(n)` names the type nowhere.
///
/// Its ground is deliberately *not* [`gap`]'s. A `memoryview` parameter is
/// impossible natively -- there is no interpreter to acquire a `Py_buffer`
/// from. Artifact-owned storage is not: `pycc_rt_buffer_f64_alloc` is a
/// plain heap allocation and `pycc_rt` links into a native executable
/// unchanged, so a native `a = ndarray(4)` would *run*. It is refused
/// because the buffer type exists to carry data across the `pycc build
/// --ext` boundary, and admitting a second, interpreter-free meaning for it
/// in native mode would commit the project to that meaning before anything
/// needs it. Reusing [`gap`]'s wording here would state a reason that is
/// false of this case, so [`producer_gap`] states the real one.
///
/// Only the *admitted* producer shape is searched, and that is sufficient
/// rather than approximate: `crates/pycc_types/src/buffer.rs` admits a
/// producer call in exactly one position -- the whole right-hand side of an
/// assignment, bare or annotated -- and refuses it with `C0001` in every
/// other position, in every artifact mode. A producer this walk does not
/// find is therefore already refused by the checker behind it.
///
/// D-244's #1129 statement (h) is honoured with the same shadow set the
/// checker uses, in two layers. The module-wide layer mirrors the checker's
/// `lookup_class`, `lookup_function` and `env.bindings` arms, read from the
/// HIR: a program's own `class ndarray`, `def ndarray`, or module-level
/// `ndarray = ...` keeps its own meaning and is not reported. A value-less
/// module-level `ndarray: int` is not such a binding; see
/// [`shadowed_producer_spellings`]. Over-refusal is the failure mode that
/// matters for that layer -- it rejects a legal program -- so it is widened
/// to the whole module rather than scoped per function.
///
/// The per-function layer added on top of it mirrors the checker's fourth
/// arm, `crate::is_local` over `function_local_names`, and is supplied by
/// `pycc_types::function_local_producer_spellings` so that the two are the
/// same computation rather than two walks that agree today: a parameter
/// named for a producer spelling, or a body that binds the spelling
/// anywhere, makes the name local throughout that function, exactly as in
/// CPython. Without this layer the gate reported `I0405` -- whose remedy is
/// "rebuild with `--ext`" -- for a program that has no producer at all and
/// that `--ext` and `pycc check` both refuse for its own, unrelated reason
/// (`T0021`), prescribing a remedy that does not apply. The layer cannot
/// under-refuse, by construction rather than by inspection: it skips a
/// spelling only where `buffer::producer_assignment_ty`'s statement-(h)
/// block declines the call, and where that block declines, nothing binds
/// `Ty::MemoryView`, so there is no artifact-owned allocation left for a
/// native executable to carry.
///
/// What this walk does *not* decide is whether the call the checker sees is
/// an allocation at all: it matches a spelling and an arity, not a length.
/// [`producer_gaps_the_check_admits`] is the semantic half, and owns why it
/// is a separate filter rather than a test this walk could make itself.
///
/// `pycc check` selects no artifact mode and so does not run this gate, the
/// same deliberate divergence [`refuse_in_native_mode`] already has: the
/// refusal's whole subject is *which artifact* is being built.
pub(crate) fn refuse_buffer_producers_in_native_mode(
    hir: &HirModule,
) -> Result<(), Vec<(usize, Diagnostic)>> {
    let shadowed = shadowed_producer_spellings(hir);
    let gaps: Vec<(usize, Diagnostic)> = hir
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let HirItem::Function {
                name, params, body, ..
            } = item
            else {
                return None;
            };
            let mut shadowed = shadowed.clone();
            shadowed.extend(pycc_types::function_local_producer_spellings(params, body));
            let callee = producer_bound_in(body, &shadowed)?;
            Some((index, producer_gap(name, callee)))
        })
        .collect();
    if gaps.is_empty() {
        return Ok(());
    }
    Err(gaps)
}

/// The [`refuse_buffer_producers_in_native_mode`] gaps the type check has
/// not already contradicted, given that check's own verdict (`check`), and
/// the whole of this gate's semantic validation.
///
/// The walk above recognizes a producer *syntactically* -- spelling, arity,
/// and D-244 #1129 statement (h) -- because it has to: it runs before the
/// type check, for the reason `src/frontend.rs`'s `resolve_frontend_native`
/// states, and the checker admits `a = ndarray(n)` in every artifact mode,
/// so there is no mode-aware verdict to wait for. What that recognition
/// cannot see is the *length expression*. `a = ndarray("x")` and
/// `a = ndarray(missing)` match the spelling and the arity and bind nothing
/// at all: the checker refuses them with `T0033` and `T0021`, which is also
/// exactly what `--ext` and `pycc check` report. Reporting `I0405` for them
/// prescribed "rebuild with `--ext`" for a program `--ext` refuses too --
/// the same defect commit 4c72b4a5 closed for statement (h)'s
/// function-local arm, in the arm next to it.
///
/// Validating the length inside this gate is not an option, and that was
/// measured rather than assumed: the length may read a module-level global
/// (`N = 4` / `a = ndarray(N)`), so any environment built here that is
/// narrower than `pycc_types`' own would infer a failure where there is
/// none and *skip* -- an artifact-owned allocation reaching a native
/// executable, the one direction this gate must never fail in. The only
/// environment wide enough is the checker's, so the gate consumes the
/// checker's verdict instead of re-deriving it.
///
/// The join is per *function*, which is what keeps it a filter rather than
/// a phase reorder: [`pycc_types::KeyedDiagnostics`] keys each diagnostic
/// by the failing item's index in `hir.items`, the same index space this
/// gate's gaps carry, so a gap is dropped exactly when that function's own
/// check failed. A function that allocates and checks clean still gets
/// `I0405`, including when a *different* function in the same program
/// fails. A function that allocates and fails its own check for an
/// unrelated reason (`a = ndarray(4)` and then `return "x"`) reports that
/// failure instead, deliberately: `--ext` refuses that program identically,
/// so `I0405`'s remedy does not apply to it either, by 4c72b4a5's own
/// criterion.
///
/// A `TopLevel` or `Module` key suppresses every gap. Such a list is never
/// mixed with `Function` keys -- `pycc_types::KeyedDiagnostics`' own
/// contract -- because a module-level failure stops the driver before it
/// checks any function body, so there is no evidence about any function and
/// the gate claims nothing.
///
/// It cannot under-refuse: a program with no producer produces no gap to
/// keep, and a gap this filter drops belongs to a function the checker has
/// already refused, so no artifact is emitted for it at all.
pub(crate) fn producer_gaps_the_check_admits(
    gaps: Vec<(usize, Diagnostic)>,
    check: &pycc_types::KeyedDiagnostics,
) -> Vec<(usize, Diagnostic)> {
    let mut failed: HashSet<usize> = HashSet::new();
    for (key, _) in check {
        match key {
            pycc_types::DiagnosticKey::Function(index) => {
                failed.insert(*index);
            }
            pycc_types::DiagnosticKey::TopLevel(_) | pycc_types::DiagnosticKey::Module => {
                return Vec::new();
            }
        }
    }
    gaps.into_iter()
        .filter(|(index, _)| !failed.contains(index))
        .collect()
}

/// The *module-wide* producer spellings the program itself rebinds, which
/// [`refuse_buffer_producers_in_native_mode`] must leave alone. The
/// function-local layer that gate adds on top of this set is
/// `pycc_types::function_local_producer_spellings`; this function does not
/// see it.
///
/// A module-level `AnnAssign` counts only when it carries an initializer.
/// `HirStmt::AnnAssign`'s `value` is an `Option`, and a value-less `ndarray:
/// int` binds nothing at run time -- the check phase records it in `declared`
/// rather than in `Environment::bindings`, so its statement-(h) test does not
/// see it either and `buffer::producer_assignment_ty` still recognizes the
/// producer. Treating the bare annotation as a shadow here while the checker
/// does not was a one-sided disagreement in the only direction that matters:
/// this gate skipped its refusal, and an artifact-owned buffer allocation
/// reached a **native** executable, past the `--ext`-only boundary
/// [`producer_gap`] exists to state.
///
/// A stdlib module alias (`import math as ndarray`) is such a binding too,
/// and is supplied by `pycc_types::imported_producer_spellings` for the same
/// composition-not-re-derivation reason the per-function layer is supplied
/// by `pycc_types::function_local_producer_spellings`. That helper's own doc
/// comment owns why the predicate must stay exactly the checker's. Like the
/// per-function layer, that arm is a mirror rather than the only thing
/// standing between the program and a native allocation:
/// [`producer_gaps_the_check_admits`] drops the gap anyway, because the
/// checker refuses the aliased call with `T0021` and the function therefore
/// fails its own check. The invariant that keeps both mirrors safe is that
/// this set stays a *subset* of what
/// `pycc_types::buffer::producer_assignment_ty` declines -- a wider set
/// would hide a gap the checker did not refuse.
///
/// A foreign import of the spelling (`import ndarray`) is the one entry of
/// that helper that is load-bearing rather than a mirror (Part 1 of #1333).
/// Since that part admits binding a CPython object in a function body,
/// `a = ndarray(n)` over a foreign `ndarray` passes its own check as an
/// ordinary CPython call, so [`producer_gaps_the_check_admits`] keeps the
/// gap, and without the entry a native build reported `I0405` for a program
/// that allocates no buffer. The entry is still inside the decline set: the
/// checker binds every foreign import as `Ty::Object` before the call is
/// classified.
fn shadowed_producer_spellings(hir: &HirModule) -> HashSet<&str> {
    let mut shadowed: HashSet<&str> = HashSet::new();
    shadowed.extend(pycc_types::imported_producer_spellings(&hir.imports));
    for (name, _) in &hir.class_defs {
        if pycc_types::is_buffer_producer_spelling(name) {
            shadowed.insert(name.as_str());
        }
    }
    for item in &hir.items {
        let name = match item {
            HirItem::Function { name, .. } => name,
            HirItem::TopLevelStmt(HirStmt::Assign { target, .. })
            | HirItem::TopLevelStmt(HirStmt::AnnAssign {
                target,
                value: Some(_),
                ..
            }) => target,
            HirItem::TopLevelStmt(_) => continue,
        };
        if pycc_types::is_buffer_producer_spelling(name) {
            shadowed.insert(name.as_str());
        }
    }
    shadowed
}

/// The spelling of the first producer call `body` binds to a name at any
/// nesting depth, or `None` when it binds none.
///
/// Exhaustive with no `_` arm for the same reason
/// `crates/pycc_hir/src/buffer_store.rs`'s walk is: a future block-carrying
/// `HirStmt` variant must be a compile error here rather than a hole this
/// gate silently stops covering.
fn producer_bound_in<'a>(body: &'a [HirStmt], shadowed: &HashSet<&str>) -> Option<&'a str> {
    body.iter().find_map(|stmt| match stmt {
        HirStmt::Assign { value, .. } => producer_callee(Some(value), shadowed),
        HirStmt::AnnAssign { value, .. } => producer_callee(value.as_ref(), shadowed),
        HirStmt::If { body, orelse, .. } => {
            producer_bound_in(body, shadowed).or_else(|| producer_bound_in(orelse, shadowed))
        }
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => producer_bound_in(body, shadowed),
        HirStmt::Match { cases, .. } => cases
            .iter()
            .find_map(|case| producer_bound_in(&case.body, shadowed)),
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => producer_bound_in(body, shadowed)
            .or_else(|| {
                handlers
                    .iter()
                    .find_map(|handler| producer_bound_in(&handler.body, shadowed))
            })
            .or_else(|| producer_bound_in(orelse, shadowed))
            .or_else(|| producer_bound_in(finalbody, shadowed)),
        // Every remaining statement either carries no nested block or binds
        // no name from a call expression, so no *admitted* producer can hide
        // in one; a producer anywhere inside them is the checker's `C0001`.
        HirStmt::ExprStmt(_)
        | HirStmt::ListCompAssign { .. }
        | HirStmt::DictCompAssign { .. }
        | HirStmt::SetCompAssign { .. }
        | HirStmt::Return(_)
        | HirStmt::DictSet { .. }
        | HirStmt::AttrSet { .. }
        | HirStmt::Delete { .. }
        | HirStmt::DeleteSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::ForeignImport { .. }
        | HirStmt::Raise { .. } => None,
    })
}

/// The producer spelling `value` calls, honouring `shadowed` and the
/// one-argument arity the checker admits.
fn producer_callee<'a>(value: Option<&'a HirExpr>, shadowed: &HashSet<&str>) -> Option<&'a str> {
    let HirExpr::Call { callee, args } = value? else {
        return None;
    };
    if args.len() != 1
        || shadowed.contains(callee.as_str())
        || !pycc_types::is_buffer_producer_spelling(callee)
    {
        return None;
    }
    Some(callee.as_str())
}

fn producer_gap(name: &str, callee: &str) -> Diagnostic {
    Diagnostic {
        code: NATIVE_MEMORYVIEW_CODE,
        severity: Severity::Error,
        message: format!(
            "`{name}` allocates buffer storage with `{callee}(n)`, which requires \
             `pycc build --ext`: a buffer exists to carry data across the CPython \
             extension-module boundary, and a native executable has no host to carry \
             it to"
        ),
        span: None,
        label: None,
        help: None,
    }
}

fn gap(name: &str, position: &str) -> Diagnostic {
    Diagnostic {
        code: NATIVE_MEMORYVIEW_CODE,
        severity: Severity::Error,
        message: format!(
            "`{name}`'s {position} requires `pycc build --ext`: a `memoryview` is a borrowed \
             view of storage a CPython interpreter owns, and a native executable embeds no \
             interpreter to acquire a buffer from"
        ),
        span: None,
        label: None,
        help: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hir(items: Vec<HirItem>) -> HirModule {
        HirModule {
            seeded_builtin_exception_classes: false,
            items,
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        }
    }

    fn func(name: &str, params: Vec<(String, Ty)>, return_ty: Ty) -> HirItem {
        HirItem::Function {
            name: name.to_string(),
            params,
            return_ty,
            body: Vec::new(),
        }
    }

    /// One export-set entry, keyed on the only field
    /// [`refuse_in_ext_mode`] reads. Every other field is inert here: the
    /// walk asks whether a name is present, never how it is carried.
    fn export(name: &str, return_ty: Ty) -> crate::ext_build::ExtExport {
        crate::ext_build::ExtExport {
            defaults: Vec::new(),
            name: name.to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: crate::ext_build::ExtReceiver::None,
            params: Vec::new(),
            param_writable: Vec::new(),
            return_ty,
            keyword_names: None,
        }
    }

    #[test]
    fn a_program_without_a_memoryview_annotation_is_admitted() {
        let admitted = hir(vec![func(
            "scale",
            vec![("x".to_string(), Ty::Float)],
            Ty::Float,
        )]);
        assert!(refuse_in_native_mode(&admitted).is_ok());
    }

    #[test]
    fn ext_mode_refuses_a_memoryview_return_type_on_a_function_no_export_walk_visits() {
        let gaps = refuse_in_ext_mode(
            &hir(vec![
                // A `memoryview` parameter is not this gate's business: the
                // read refusal in `crates/pycc_types` already closes it.
                func("total", vec![("v".to_string(), Ty::MemoryView)], Ty::Int),
                // A non-function item is skipped: only a signature can name a
                // return type at all.
                HirItem::TopLevelStmt(pycc_hir::HirStmt::Assign {
                    target: "n".to_string(),
                    value: pycc_hir::HirExpr::IntLiteral(1),
                }),
                func("_make", Vec::new(), Ty::MemoryView),
                func("Buf.view", Vec::new(), Ty::MemoryView),
            ]),
            // Part 2b of #1142 (#1164): an empty export set, so nothing here
            // is exempt -- including `Buf.view`, which #1174 admits when it
            // *is* exported. The sibling test below pins that direction.
            &[],
        )
        .expect_err("a `memoryview` return type is refused under --ext");
        let messages: Vec<&str> = gaps.iter().map(|gap| gap.message.as_str()).collect();
        assert_eq!(gaps.len(), 2, "{messages:?}");
        assert!(
            gaps.iter()
                .all(|gap| gap.code == "C0001" && gap.span.is_none()),
            "{messages:?}"
        );
        assert!(
            messages[0].contains("`_make`'s return type is a buffer"),
            "{messages:?}"
        );
        assert!(
            messages[1].contains("`Buf.view`'s return type is a buffer"),
            "{messages:?}"
        );
    }

    #[test]
    fn ext_mode_admits_a_program_that_returns_no_memoryview() {
        let admitted = hir(vec![func(
            "total",
            vec![("v".to_string(), Ty::MemoryView)],
            Ty::Float,
        )]);
        assert!(refuse_in_ext_mode(&admitted, &[]).is_ok());
    }

    /// #1174: the exemption is *exported*, with no module-level half left.
    ///
    /// `make` is exported and dot-free and `Buf.view` is exported and dotted;
    /// both are admitted, because `crate::buffer::
    /// refuse_buffer_returning_method` now closes the four method-resolution
    /// exits that `pycc_types`' buffer-returning-call interception did not
    /// cover, so `g.view()` inside the artifact is refused at the *call*
    /// rather than the declaration. `_make` is unexported and is still
    /// refused -- membership in the export set is the whole test now.
    #[test]
    fn ext_mode_admits_every_exported_buffer_return_and_nothing_else() {
        let exports = vec![
            export("make", Ty::MemoryView),
            export("Buf.view", Ty::MemoryView),
        ];
        let program = hir(vec![
            func("make", Vec::new(), Ty::MemoryView),
            func("_make", Vec::new(), Ty::MemoryView),
            func("Buf.view", Vec::new(), Ty::MemoryView),
        ]);
        let gaps = refuse_in_ext_mode(&program, &exports)
            .expect_err("an unexported buffer return is still refused");
        let messages: Vec<&str> = gaps.iter().map(|gap| gap.message.as_str()).collect();
        assert_eq!(gaps.len(), 1, "{messages:?}");
        assert!(
            messages[0].contains("`_make`'s return type is a buffer"),
            "{messages:?}"
        );
        // Two-directional: the admitted export is not merely absent from the
        // gap list, it is admitted when it is the only item in the program.
        assert!(
            refuse_in_ext_mode(
                &hir(vec![func("make", Vec::new(), Ty::MemoryView)]),
                &exports,
            )
            .is_ok()
        );
        // #1174's own direction: the *method* is admitted on its own too,
        // which is the half that was refused before this change.
        assert!(
            refuse_in_ext_mode(
                &hir(vec![func("Buf.view", Vec::new(), Ty::MemoryView)]),
                &exports,
            )
            .is_ok()
        );
        // And a *name* match alone is not the test: an export set holding a
        // different name leaves `make` refused.
        assert!(
            refuse_in_ext_mode(
                &hir(vec![func("make", Vec::new(), Ty::MemoryView)]),
                &[export("other", Ty::MemoryView)],
            )
            .is_err()
        );
    }

    #[test]
    fn every_memoryview_signature_is_reported_not_only_the_first() {
        let gaps = refuse_in_native_mode(&hir(vec![
            func("total", vec![("xs".to_string(), Ty::MemoryView)], Ty::Float),
            func("plain", vec![("x".to_string(), Ty::Int)], Ty::Int),
            // The return position is the second half of the gate: it is a
            // `C0003` capability gap under `--ext`, but in native mode it
            // is the same refusal a parameter gets.
            func("view", Vec::new(), Ty::MemoryView),
        ]))
        .expect_err("a memoryview annotation is refused in native mode");
        let messages: Vec<&str> = gaps.iter().map(|(_, gap)| gap.message.as_str()).collect();
        assert_eq!(gaps.len(), 2, "{messages:?}");
        assert!(
            gaps.iter()
                .all(|(_, gap)| gap.code == NATIVE_MEMORYVIEW_CODE && gap.span.is_none()),
            "{messages:?}"
        );
        assert!(messages[0].contains("`xs: memoryview`"), "{messages:?}");
        assert!(messages[1].contains("`-> memoryview`"), "{messages:?}");
        assert!(messages[0].contains("--ext"), "{messages:?}");
        // Each gap carries its own index in `hir.items` -- not its position
        // among the offending ones -- because that is what the driver joins
        // against the program's per-file item bounds to name the owning file.
        let positions: Vec<usize> = gaps.iter().map(|(index, _)| *index).collect();
        assert_eq!(positions, vec![0, 2], "{messages:?}");
    }

    // ---------------------------------------------------------------------
    // Part 2a of #1142 (#1165): the body-level artifact-owned buffer gate.
    // ---------------------------------------------------------------------

    /// A minimal non-generic, non-protocol class, for the statement-(h)
    /// shadow test below.
    fn plain_class_def(name: &str) -> pycc_hir::HirClassDef {
        pycc_hir::HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: name.to_string(),
            bases: Vec::new(),
            mro: vec![name.to_string()],
            attrs: Vec::new(),
            methods: Vec::new(),
            type_param: None,
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            is_enum: false,
            implicit_object_init: true,
            method_defaults: Vec::new(),
            enum_members: Vec::new(),
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        }
    }

    /// `a = <callee>(4)`.
    fn alloc(callee: &str) -> HirStmt {
        HirStmt::Assign {
            target: "a".to_string(),
            value: HirExpr::Call {
                callee: callee.to_string(),
                args: vec![HirExpr::IntLiteral(4)],
            },
        }
    }

    /// `def <name>(): <body>`.
    fn body_func(name: &str, body: Vec<HirStmt>) -> HirItem {
        HirItem::Function {
            name: name.to_string(),
            params: vec![],
            return_ty: Ty::None,
            body,
        }
    }

    /// Both spellings, both binding forms, and a message whose ground is the
    /// missing *host* rather than the missing interpreter: the allocation
    /// itself would work natively, which is exactly why the refusal needs
    /// its own wording instead of [`gap`]'s.
    #[test]
    fn allocating_a_buffer_natively_is_refused_in_its_own_words() {
        for stmt in [
            alloc("ndarray"),
            alloc("NDArray"),
            HirStmt::AnnAssign {
                target: "a".to_string(),
                annotation: Ty::MemoryView,
                value: Some(HirExpr::Call {
                    callee: "ndarray".to_string(),
                    args: vec![HirExpr::IntLiteral(4)],
                }),
                is_final: false,
            },
        ] {
            let module = hir(vec![body_func("f", vec![stmt])]);
            let gaps = refuse_buffer_producers_in_native_mode(&module).unwrap_err();
            assert_eq!(gaps.len(), 1);
            assert_eq!(gaps[0].0, 0);
            assert_eq!(gaps[0].1.code, NATIVE_MEMORYVIEW_CODE);
            assert!(gaps[0].1.message.contains("`f`"), "{:?}", gaps[0].1);
            assert!(gaps[0].1.message.contains("pycc build --ext"));
            assert!(gaps[0].1.message.contains("extension-module boundary"));
            assert!(gaps[0].1.span.is_none());
        }
    }

    /// A program that allocates nowhere is untouched, and so is a top-level
    /// statement (module scope is `pycc_types`' own refusal, in every mode).
    #[test]
    fn a_program_that_allocates_no_buffer_is_admitted_natively() {
        assert!(refuse_buffer_producers_in_native_mode(&hir(vec![])).is_ok());
        assert!(
            refuse_buffer_producers_in_native_mode(&hir(vec![
                body_func("f", vec![HirStmt::Return(None)]),
                HirItem::TopLevelStmt(alloc("ndarray")),
            ]))
            .is_ok()
        );
    }

    /// Every nesting shape the walk recurses through reaches the producer,
    /// and every shape it deliberately does not recurse through is left to
    /// the checker's own `C0001`.
    #[test]
    fn the_walk_reaches_a_producer_at_every_nesting_depth() {
        let nested: Vec<Vec<HirStmt>> = vec![
            vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![alloc("ndarray")],
                orelse: vec![],
            }],
            vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![],
                orelse: vec![alloc("ndarray")],
            }],
            vec![HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: vec![alloc("ndarray")],
            }],
            vec![HirStmt::Try {
                body: vec![alloc("ndarray")],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![],
            }],
            vec![HirStmt::Try {
                body: vec![],
                handlers: vec![],
                orelse: vec![alloc("ndarray")],
                finalbody: vec![],
            }],
            vec![HirStmt::Try {
                body: vec![],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![alloc("ndarray")],
            }],
            vec![HirStmt::Try {
                body: vec![],
                handlers: vec![pycc_hir::HirExceptHandler {
                    exc_type: None,
                    name: None,
                    body: vec![alloc("ndarray")],
                }],
                orelse: vec![],
                finalbody: vec![],
            }],
            vec![HirStmt::Match {
                subject: HirExpr::IntLiteral(0),
                cases: vec![pycc_hir::HirMatchCase {
                    pattern: pycc_hir::HirPattern::Wildcard,
                    guard: None,
                    body: vec![alloc("ndarray")],
                }],
            }],
            vec![HirStmt::ForRange {
                var: "i".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(1),
                step: HirExpr::IntLiteral(1),
                body: vec![alloc("ndarray")],
            }],
            // A value-less annotated declaration binds no producer, and a
            // nested block that holds none leaves the walk at `None`.
            vec![
                HirStmt::AnnAssign {
                    target: "a".to_string(),
                    annotation: Ty::Int,
                    value: None,
                    is_final: false,
                },
                alloc("ndarray"),
            ],
        ];
        for body in nested {
            let module = hir(vec![body_func("f", body.clone())]);
            assert!(
                refuse_buffer_producers_in_native_mode(&module).is_err(),
                "{body:?}"
            );
        }
        // Not an assignment's right-hand side, so not this gate's subject.
        let module = hir(vec![body_func(
            "f",
            vec![
                HirStmt::ExprStmt(HirExpr::Call {
                    callee: "ndarray".to_string(),
                    args: vec![HirExpr::IntLiteral(4)],
                }),
                HirStmt::Return(None),
            ],
        )]);
        assert!(refuse_buffer_producers_in_native_mode(&module).is_ok());
    }

    /// The arity the checker admits is the arity this gate claims: a
    /// two-argument call is `pycc_types`' `C0001`, not an `I0405`.
    #[test]
    fn a_producer_with_the_wrong_arity_is_not_this_gates_subject() {
        let module = hir(vec![body_func(
            "f",
            vec![HirStmt::Assign {
                target: "a".to_string(),
                value: HirExpr::Call {
                    callee: "ndarray".to_string(),
                    args: vec![HirExpr::IntLiteral(4), HirExpr::IntLiteral(5)],
                },
            }],
        )]);
        assert!(refuse_buffer_producers_in_native_mode(&module).is_ok());
    }

    /// D-244 #1129 statement (h): the program's own binding wins, through
    /// each of the three tables the checker consults. Over-refusal is the
    /// failure that matters here -- it rejects a legal native program.
    #[test]
    fn a_program_that_shadows_the_spelling_is_not_refused() {
        let own_function = hir(vec![
            HirItem::Function {
                name: "ndarray".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(None)],
            },
            body_func("f", vec![alloc("ndarray")]),
        ]);
        assert!(refuse_buffer_producers_in_native_mode(&own_function).is_ok());

        let own_binding = hir(vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "ndarray".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            body_func("f", vec![alloc("ndarray")]),
        ]);
        assert!(refuse_buffer_producers_in_native_mode(&own_binding).is_ok());

        let own_annotated_binding = hir(vec![
            HirItem::TopLevelStmt(HirStmt::AnnAssign {
                target: "ndarray".to_string(),
                annotation: Ty::Int,
                value: Some(HirExpr::IntLiteral(1)),
                is_final: false,
            }),
            body_func("f", vec![alloc("ndarray")]),
        ]);
        assert!(refuse_buffer_producers_in_native_mode(&own_annotated_binding).is_ok());

        let mut own_class = hir(vec![body_func("f", vec![alloc("NDArray")])]);
        own_class
            .class_defs
            .push(("NDArray".to_string(), plain_class_def("NDArray")));
        assert!(refuse_buffer_producers_in_native_mode(&own_class).is_ok());

        // A foreign import of the spelling (Part 1 of #1333): the checker
        // admits `a = ndarray(4)` as a CPython call, so this gate must not
        // report an allocation the program does not make.
        let mut foreign_import = hir(vec![body_func("f", vec![alloc("ndarray")])]);
        foreign_import
            .imports
            .push(pycc_hir::ImportBinding::Foreign {
                local_name: "ndarray".to_string(),
                module_path: "ndarray".to_string(),
                from: None,
                site: pycc_hir::ForeignImportSite::Item(0),
                span: pycc_diag::Span::new(0, 0),
            });
        assert!(refuse_buffer_producers_in_native_mode(&foreign_import).is_ok());

        // A foreign import of some *other* name shadows nothing.
        let mut other_foreign = hir(vec![body_func("f", vec![alloc("ndarray")])]);
        other_foreign
            .imports
            .push(pycc_hir::ImportBinding::Foreign {
                local_name: "numpy".to_string(),
                module_path: "numpy".to_string(),
                from: None,
                site: pycc_hir::ForeignImportSite::Item(0),
                span: pycc_diag::Span::new(0, 0),
            });
        assert!(refuse_buffer_producers_in_native_mode(&other_foreign).is_err());

        // An unrelated top-level statement contributes no shadow and stops
        // no refusal.
        let unrelated = hir(vec![
            HirItem::TopLevelStmt(HirStmt::Return(None)),
            body_func("f", vec![alloc("ndarray")]),
        ]);
        assert!(refuse_buffer_producers_in_native_mode(&unrelated).is_err());
    }

    /// Every function that allocates is named, not only the first: one build
    /// reports the whole list, exactly as the signature walk does.
    #[test]
    fn every_allocating_function_is_reported() {
        let module = hir(vec![
            body_func("f", vec![alloc("ndarray")]),
            body_func("g", vec![HirStmt::Return(None)]),
            body_func("h", vec![alloc("NDArray")]),
        ]);
        let gaps = refuse_buffer_producers_in_native_mode(&module).unwrap_err();
        assert_eq!(
            gaps.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            vec![0, 2]
        );
    }
}
