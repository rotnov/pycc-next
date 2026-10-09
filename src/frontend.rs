//! The driver's frontend seam: runs a program through the parser, checked
//! HIR lowering, whole-program linking, and the type checker, and renders
//! whatever diagnostics the first failing pass collected (#864 Part 1,
//! D-217; Part 2's per-item HIR collection, D-219, and Part 3's
//! per-function type-checker collection, D-220, flow through the same
//! payload).
//!
//! Since #898 (Part 1 of #881, D-222) a "program" is one or more files:
//! `src/modules.rs` loads and lowers the entry file's whole import
//! closure, `pycc_hir::link` concatenates the modules into one
//! `HirModule`, and the type checker's keyed diagnostics
//! (`pycc_types::DiagnosticKey`) are mapped back to the file that owns
//! the item they came from.
//!
//! Extracted from `src/main.rs` (AGENTS.md's oversized-file rule) when the
//! failure payload became a `Vec<Diagnostic>`; `main.rs` keeps the command
//! dispatch and calls into here.

use crate::cli::ErrorFormat;
pub(crate) use crate::foreign_import::{EmbedHost, NeedsInterpreter};
use crate::interop_policy::{self, InteropCli};
use crate::modules::{self, DiscoveredManifest, LoadedProgram, RelativeImports};
use pycc_diag::Diagnostic;
use pycc_hir::{HirModule, LinkInput};
use pycc_types::DiagnosticKey;
use std::path::Path;

/// The `__name__` value every non-`--ext` compilation gives its entry module
/// (W0 of #882, #1156). CPython names the module it runs as a script
/// `__main__`, which is what makes the `if __name__ == "__main__":` idiom
/// take its true branch in a `pycc build`/`pycc run` artifact; `pycc check`
/// uses the same value so a program checks exactly as it builds.
pub(crate) const NATIVE_MODULE_NAME: &str = "__main__";

/// Every diagnostic collected for one file of the program, with the source
/// they render against. `diagnostics` is never empty: a file with none is
/// never added to [`FrontendFailure::Compile`].
pub(crate) struct FileDiagnostics {
    pub(crate) path: String,
    pub(crate) source: String,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

/// Why a frontend pass could not produce a typed program.
pub(crate) enum FrontendFailure {
    /// A file could not be read or decoded (CLI_SPEC.md's exit-2
    /// invocation/environment class); never subject to `--error-format`.
    /// `path` names the file that failed, which since #898 is not
    /// necessarily the file named on the command line -- an unreadable
    /// *dependency* fails the same way, under its own path.
    Input { path: String, message: String },
    /// A frontend pass rejected the program. Each entry holds every
    /// diagnostic the *first failing pass* collected for that file, in
    /// that pass's own collection order (the parser: ruff's discovery
    /// order, see `pycc_parser::parse_all`; HIR lowering: per top-level
    /// item in source order, the item's own diagnostic when it fails plus
    /// one enum-call `C0001` per enum-class call inside it that the scan
    /// can attribute (a shadowed or rebound name falls through to the type
    /// checker's span-less guard), with cascades of an earlier skipped item
    /// suppressed, see `pycc_hir::lower_module`, D-219 and D-233; linking:
    /// the first cross-module conflict; the type checker: one per failing
    /// item, solver-first per function; a pre-check or module-level solver
    /// failure is reported alone, otherwise the checker's entries for
    /// functions the solver did not flag follow the solver's -- see
    /// `pycc_types::check_all_keyed` and D-220). Files appear in the
    /// program's own dependency order, entry last.
    ///
    /// Invariant: `files` is non-empty and every `diagnostics` is
    /// non-empty. Every constructor below either wraps one non-empty
    /// diagnostic list or forwards `pycc_parser::parse_all`'s,
    /// `pycc_hir::lower_module`'s/`link`'s/`finalize`'s, or
    /// `pycc_types::check_all_keyed`/`check_and_resolve_all_keyed`'s `Err`,
    /// all non-empty by construction (proven by those crates' unit tests).
    /// No runtime assertion guards it: an `assert!` would add an
    /// uncoverable in-crate region under D-014's 100%-region gate. (Should
    /// the invariant ever break, `render_all`'s loops would print nothing
    /// and `check` would exit 1 silently -- a contract violation of
    /// CLI_SPEC.md's "exit 1 means at least one diagnostic", which is
    /// exactly why construction, not rendering, is what guarantees
    /// non-emptiness.)
    Compile { files: Vec<FileDiagnostics> },
}

impl FrontendFailure {
    pub(crate) fn input(path: String, message: String) -> Self {
        Self::Input { path, message }
    }

    /// The single-file compile failure: every diagnostic belongs to `path`.
    pub(crate) fn compile(path: &str, source: &str, diagnostics: Vec<Diagnostic>) -> Self {
        Self::Compile {
            files: vec![FileDiagnostics {
                path: path.to_string(),
                source: source.to_string(),
                diagnostics,
            }],
        }
    }
}

/// The per-file sources of a loaded program, kept alongside the linked
/// module so a diagnostic can be rendered against the file that owns it.
struct ProgramSources {
    files: Vec<(String, String)>,
    /// `bounds[i]` is the number of `HirModule` items contributed by files
    /// `0..=i`, so `bounds.partition_point(|end| *end <= index)` names the
    /// file that owns item `index`. An index past the last bound belongs to
    /// an item `link`/`finalize` appended for the whole program (the seeded
    /// builtin exception classes, `Exception.__init__`), which is attributed
    /// to the entry file.
    bounds: Vec<usize>,
    /// `import_bounds[i]` is the number of `ImportBinding`s contributed by
    /// files `0..=i`, the import-table analogue of `bounds` (Part 1 of
    /// #1026). Read by [`Self::owner_of_import`].
    import_bounds: Vec<usize>,
    /// `class_bounds[i]` is the number of `class_defs` entries contributed
    /// by files `0..=i`, the class-table analogue of `bounds`. Read by
    /// [`Self::owner_of_class`].
    ///
    /// It is *not* each module's own `hir.class_defs.len()`:
    /// `pycc_hir::link` drops a module's seeded builtin exception classes
    /// as it concatenates and appends one set for the whole program at the
    /// back, so a module's contribution is its non-seeded entry count. That
    /// trailing program-wide block is past the last bound and falls to the
    /// entry file, exactly as `bounds` treats `finalize`'s appended items.
    class_bounds: Vec<usize>,
}

impl ProgramSources {
    fn entry(&self) -> usize {
        self.files.len() - 1
    }

    fn owner(&self, key: DiagnosticKey) -> usize {
        match key.item_index() {
            Some(index) => self.owner_of_item(index),
            None => self.entry(),
        }
    }

    /// The file that owns item `index` of the linked program.
    fn owner_of_item(&self, index: usize) -> usize {
        self.bounds
            .partition_point(|end| *end <= index)
            .min(self.entry())
    }

    /// The file that owns import `position` of the linked program's import
    /// table (Part 1 of #1026).
    ///
    /// Same shape as [`Self::owner_of_item`] against the import bounds
    /// instead of the item bounds. An import's own item index cannot
    /// serve here: it is the item count at the moment the `import` lowered,
    /// so a trailing import in one file and a leading import in the next
    /// record the same linked index. The import table has no such boundary
    /// ambiguity -- `pycc_hir::link` concatenates each module's imports in
    /// the same file order as its items.
    ///
    /// The `.min(entry())` clamp is unreachable here (the last import bound
    /// *is* the linked import count, so `partition_point` can never exceed
    /// `entry()`); it is kept for shape parity with `owner_of_item`, where
    /// `finalize`'s appended program-wide items make it load-bearing.
    fn owner_of_import(&self, position: usize) -> usize {
        self.import_bounds
            .partition_point(|end| *end <= position)
            .min(self.entry())
    }

    /// The file that owns class `index` of the linked program's class table
    /// (Part 1 of #1027).
    ///
    /// Same shape as [`Self::owner_of_item`] against the class bounds. The
    /// `.min(entry())` clamp is load-bearing here for the same reason it is
    /// there: `pycc_hir::link` appends the seeded builtin exception classes
    /// once, after every module's own contribution, so their indices sit
    /// past the last bound.
    fn owner_of_class(&self, index: usize) -> usize {
        self.class_bounds
            .partition_point(|end| *end <= index)
            .min(self.entry())
    }

    /// Groups keyed diagnostics into per-file payloads, in program order.
    fn group(&self, keyed: Vec<(usize, Diagnostic)>) -> FrontendFailure {
        let mut files: Vec<FileDiagnostics> = self
            .files
            .iter()
            .map(|(path, source)| FileDiagnostics {
                path: path.clone(),
                source: source.clone(),
                diagnostics: Vec::new(),
            })
            .collect();
        for (index, diagnostic) in keyed {
            files[index].diagnostics.push(diagnostic);
        }
        files.retain(|file| !file.diagnostics.is_empty());
        FrontendFailure::Compile { files }
    }
}

/// Loads and links the entry file's whole import closure into the single
/// `HirModule` the rest of the pipeline consumes.
///
/// `module_name` is the `__name__` value the entry module is compiled with
/// (W0 of #882, #1156): `"__main__"` for `pycc check` and for a native
/// `pycc build`/`pycc run`, and the extension module's own name for a
/// `pycc build --ext`. It reaches `pycc_hir::lower_module` through
/// `modules::load`, which gives it to the entry module alone.
///
/// The third value is the `pycc.toml` source-root discovery parsed, if any,
/// which the interop policy reads (#1224); `--ext` drops it.
///
/// `relative_imports` is how the entry module's relative imports (#1366),
/// and since #1382 its absolute imports rooted at its own top-level package,
/// resolve: [`RelativeImports::Project`] everywhere but a `pycc build --ext
/// --foreign-relative-imports`.
///
/// `ext_module` is whether the program is compiled into an `ext` artifact
/// (D-258 rule 1, #1397): `true` only from [`resolve_frontend_with`], the
/// `pycc build --ext` frontend, so `pycc check` and every native build keep
/// today's `Any`/`object`/container refusals byte-identical.
fn link_frontend(
    path: &Path,
    module_name: Option<&str>,
    relative_imports: RelativeImports,
    ext_module: bool,
) -> Result<(HirModule, ProgramSources, Option<DiscoveredManifest>), FrontendFailure> {
    let program: LoadedProgram =
        modules::load_with(path, module_name, relative_imports, ext_module)?;
    let manifest = program.manifest;
    let mut files = Vec::with_capacity(program.modules.len());
    let mut bounds = Vec::with_capacity(program.modules.len());
    let mut import_bounds = Vec::with_capacity(program.modules.len());
    let mut class_bounds = Vec::with_capacity(program.modules.len());
    let mut inputs = Vec::with_capacity(program.modules.len());
    let mut total = 0;
    let mut imports_total = 0;
    let mut classes_total = 0;
    // #1244: `pycc_hir::link` refuses a module-scope `del x` while another
    // module mentions `x`. Lowering leaves each module's mentions unset,
    // since the walk would cost every build; they are computed here, from
    // the kept source, only for a multi-module program that deletes a name.
    let needs_mentions = program.modules.len() > 1
        && program
            .modules
            .iter()
            .any(|loaded| !loaded.module.deleted_top_level.is_empty());
    for mut loaded in program.modules {
        if needs_mentions {
            let parsed = pycc_parser::parse_all(&loaded.source)
                .expect("a module that lowered once parses again");
            loaded.module.mentioned_names = Some(pycc_hir::mentioned_names(&parsed));
        }
        total += loaded.module.hir.items.len();
        bounds.push(total);
        imports_total += loaded.module.hir.imports.len();
        import_bounds.push(imports_total);
        // Mirrors `pycc_hir::link`'s own filter exactly: a seeded module's
        // builtin exception classes are dropped as it concatenates, so
        // counting this module's raw `class_defs` would misalign every
        // later file's bound.
        let seeded = loaded.module.hir.seeded_builtin_exception_classes;
        classes_total += loaded
            .module
            .hir
            .class_defs
            .iter()
            .filter(|(name, _)| !(seeded && pycc_hir::is_builtin_exception_class(name)))
            .count();
        class_bounds.push(classes_total);
        files.push((loaded.display_path.clone(), loaded.source));
        inputs.push(LinkInput {
            display_path: loaded.display_path,
            module: loaded.module,
        });
    }
    let sources = ProgramSources {
        files,
        bounds,
        import_bounds,
        class_bounds,
    };
    let linked = pycc_hir::link(inputs).map_err(|keyed| sources.group(keyed))?;
    let hir = pycc_hir::finalize(linked).map_err(|diagnostics| {
        let entry = sources.entry();
        sources.group(diagnostics.into_iter().map(|d| (entry, d)).collect())
    })?;
    Ok((hir, sources, manifest))
}

/// `pycc check`'s frontend: link, resolve the interop policy, type-check,
/// then the policy gate (#1224).
///
/// The policy is resolved -- and a malformed `[interop]` table refused with
/// exit 2 -- right after linking, at the same point
/// [`resolve_frontend_native`] does it, so `check` and `build` agree even on
/// a program with type errors. A type error still wins over an `I0402`, as
/// it wins over an import gap in a build. `check` selects no artifact mode,
/// so it never emits `I0403`: only whether the policy admits each
/// CPython-backed import is checked here.
pub(crate) fn check_frontend(path: &Path, interop: InteropCli) -> Result<(), FrontendFailure> {
    let (hir, sources, manifest) = link_frontend(
        path,
        Some(NATIVE_MODULE_NAME),
        RelativeImports::Project,
        false,
    )?;
    let policy = interop_policy::resolve_for_program(&hir, manifest.as_ref(), interop)?;
    pycc_types::check_all_keyed(&hir).map_err(|keyed| sources.group(attribute(&sources, keyed)))?;
    let gaps = interop_policy::policy_gaps(&hir, &policy);
    if gaps.is_empty() {
        return Ok(());
    }
    Err(sources.group(
        gaps.into_iter()
            .map(|(position, diagnostic)| (sources.owner_of_import(position), diagnostic))
            .collect(),
    ))
}

/// `pycc lock`'s frontend (the pycc.lock decision entry, D-249, rule 6):
/// [`check_frontend`] without the type check. The lock needs only the
/// linked program's CPython-backed imports and whether the effective
/// interop policy admits each one, so a policy-rejected root is `I0402`
/// here exactly as in `check`, and a malformed `[interop]` table is the
/// same exit-2 input failure.
pub(crate) fn lock_frontend(
    path: &Path,
    interop: InteropCli,
) -> Result<HirModule, FrontendFailure> {
    let (hir, sources, manifest) = link_frontend(
        path,
        Some(NATIVE_MODULE_NAME),
        RelativeImports::Project,
        false,
    )?;
    let policy = interop_policy::resolve_for_program(&hir, manifest.as_ref(), interop)?;
    let gaps = interop_policy::policy_gaps(&hir, &policy);
    if gaps.is_empty() {
        return Ok(hir);
    }
    Err(sources.group(
        gaps.into_iter()
            .map(|(position, diagnostic)| (sources.owner_of_import(position), diagnostic))
            .collect(),
    ))
}

/// Link and type-check the entry file's program, with relative imports
/// resolved as project imports (D-222). Test-only: every production caller
/// chooses the relative-import mode through [`resolve_frontend_with`].
#[cfg(test)]
pub(crate) fn resolve_frontend(
    path: &Path,
    module_name: Option<&str>,
) -> Result<HirModule, FrontendFailure> {
    resolve_linked(path, module_name, RelativeImports::Project, false)
}

/// Link and type-check the entry file's program, the `pycc build --ext`
/// frontend. `relative_imports` is how the entry module's relative imports,
/// and since #1382 its absolute imports rooted at its own top-level package,
/// resolve: `--foreign-relative-imports` passes
/// [`RelativeImports::ForeignFromEntry`] (#1366), every other caller
/// [`RelativeImports::Project`].
pub(crate) fn resolve_frontend_with(
    path: &Path,
    module_name: Option<&str>,
    relative_imports: RelativeImports,
) -> Result<HirModule, FrontendFailure> {
    resolve_linked(path, module_name, relative_imports, true)
}

/// Links and type-checks the entry file's program; `ext_module` is
/// [`link_frontend`]'s (D-258, #1397).
fn resolve_linked(
    path: &Path,
    module_name: Option<&str>,
    relative_imports: RelativeImports,
    ext_module: bool,
) -> Result<HirModule, FrontendFailure> {
    let (hir, sources, _manifest) = link_frontend(path, module_name, relative_imports, ext_module)?;
    pycc_types::check_and_resolve_all_keyed(&hir)
        .map_err(|keyed| sources.group(attribute(&sources, keyed)))
}

/// [`resolve_frontend_with`] plus the native-mode artifact gates, for a
/// `pycc build` without `--ext`: the foreign-import gate (Part 1 of #1026,
/// narrowed by Part 1 of #1028) and the `memoryview`-annotation gate
/// (Part 1 of #1027).
///
/// The foreign-import gate is now a classifier
/// (`crate::foreign_import::classify_for_native_build`): a program whose
/// foreign imports are all embeddable on `host` (every root but an excluded
/// standard-library one; a third-party root is bundled from `pycc.lock`,
/// #1242) is admitted with [`NeedsInterpreter`]`(true)` and built as an
/// embedded executable; any other foreign import is still `I0403`, reported
/// exactly where the former all-foreign refusal was. The `memoryview` and
/// buffer-producer gates keep applying to an embedded build unchanged,
/// because an embedded executable is still not an extension module.
///
/// The gate runs here rather than in `main.rs` for one reason: only this
/// module holds the `ProgramSources` that says which *file* an import
/// belongs to, and a foreign import in a dependency must be reported
/// against that dependency, not against the entry path (PR 1c of #1080
/// review finding 2).
///
/// Order matters three times, and the gates resolve it differently.
///
/// All three are *computed* before the type check, against the linked HIR whose
/// item indices still line up with the per-file bounds --
/// `check_and_resolve_all_keyed` runs monomorphization and enum lowering,
/// which rewrite the item list and recompute those positions.
///
/// The foreign-import gate is *reported* after, so a program with both a
/// type error and a foreign import still reports the type error first,
/// exactly as the former `main.rs` call site did -- when no `memoryview`
/// gap fires in the same program. When one does, the early return below
/// reports the collected gaps and the type check never runs at all, so
/// there is no type error to order against. The `memoryview` gate
/// cannot be: `crates/pycc_types`'s `reject_memoryview_read` refuses every
/// *use* of a `memoryview`-typed name with `C0001` except the `b[i]`
/// element load Part 2 of #1027 admits, so a body that so much as reads
/// its own parameter any other way fails the type check first, and the
/// signature-level `I0405` that `docs/RUNTIME.md` and D-244 promise for
/// "a `memoryview` in a signature in a build without `--ext`" would never
/// be emitted (#1115 review round 5). It is therefore reported *before*
/// the type check -- together with any foreign-import gap found in the
/// same program, so prioritizing it never swallows an `I0403` that would
/// otherwise have been reported.
///
/// The buffer-producer gate is the third case (#1165). Its walk is computed
/// before the type check like the others, because the checker admits
/// `a = ndarray(n)` in every artifact mode and so never produces a
/// mode-aware verdict to wait for. Its *report* is then filtered by that
/// same check, per function, through
/// `crate::memoryview_mode::producer_gaps_the_check_admits`: a producer
/// whose length the checker refuses is not an allocation, and reporting
/// `I0405` for it prescribed a remedy `--ext` does not satisfy either.
/// That filter is unconditional -- it applies on every path this function
/// can report a producer gap from, including the `memoryview` early return
/// above, which obtains a verdict *purely* to filter with and discards the
/// verdict's own diagnostics.
///
/// The inventory, because a missed site is exactly how this defect reached
/// review twice: `resolve_frontend_native` has three producer-gap report
/// sites -- the `refused_a_buffer` early return (filtered), the type-check
/// `Err` arm (filtered), and the `Ok(resolved)` tail (provably needs no
/// filter, because reaching it means the check returned `Ok` and so keys no
/// error to any function).
///
/// `pycc check` selects no artifact mode and so runs neither gate:
/// [`check_frontend`] reports the `C0001` read refusal there, which is
/// correct, because `I0405`'s contract is scoped to a *build* without
/// `--ext`.
///
/// The interop policy (#1224) is resolved first, right after linking, so a
/// malformed `[interop]` table is exit 2 before any gate runs. It then
/// feeds the foreign-import classification, which gives each CPython-backed
/// import the policy rejects an `I0402` *instead of* any `I0403`: the
/// `import_gaps` vector carries both codes through the three report paths
/// below unchanged.
pub(crate) fn resolve_frontend_native(
    path: &Path,
    host: EmbedHost,
    interop: InteropCli,
) -> Result<(HirModule, NeedsInterpreter), FrontendFailure> {
    let (hir, sources, manifest) = link_frontend(
        path,
        Some(NATIVE_MODULE_NAME),
        RelativeImports::Project,
        false,
    )?;
    let policy = interop_policy::resolve_for_program(&hir, manifest.as_ref(), interop)?;
    let import_gaps = crate::foreign_import::classify_for_native_build(&hir, host, &policy);
    // Keyed by *item* index rather than import position: a `memoryview`
    // annotation lives on an `HirItem::Function`, not in the import table,
    // so it resolves to its owning file through the item bounds.
    let memoryview_gaps = crate::memoryview_mode::refuse_in_native_mode(&hir);
    // Keyed by *class* index: a protocol method's signature is a
    // `ProtocolMember::Method` in `hir.class_defs`, never an
    // `HirItem::Function`, so it resolves through the class bounds instead.
    let protocol_gaps = crate::memoryview_mode::refuse_protocol_methods_in_native_mode(&hir);
    // Keyed by *item* index like the signature walk, and *computed* in the
    // same pre-check pass for a different reason (#1165): the checker has no
    // artifact-mode awareness and *admits* `a = ndarray(n)`, so there is no
    // mode-aware verdict to wait for -- running the walk afterwards would
    // mean never running it at all.
    let producer_gaps = crate::memoryview_mode::refuse_buffer_producers_in_native_mode(&hir);
    let mut keyed: Vec<(usize, Diagnostic)> = Vec::new();
    // `Ok` carries the embed verdict for the `Ok(resolved)` tail; an import
    // gap only ever reaches a path that returns `Err`, so the placeholder
    // below is never observed.
    let needs_interpreter = match import_gaps {
        Ok(needs) => needs,
        Err(gaps) => {
            keyed.extend(
                gaps.into_iter()
                    .map(|(position, diagnostic)| (sources.owner_of_import(position), diagnostic)),
            );
            NeedsInterpreter(false)
        }
    };
    let refused_a_buffer = memoryview_gaps.is_err() || protocol_gaps.is_err();
    if let Err(gaps) = memoryview_gaps {
        keyed.extend(
            gaps.into_iter()
                .map(|(index, diagnostic)| (sources.owner_of_item(index), diagnostic)),
        );
    }
    if let Err(gaps) = protocol_gaps {
        keyed.extend(
            gaps.into_iter()
                .map(|(index, diagnostic)| (sources.owner_of_class(index), diagnostic)),
        );
    }
    let producer_gaps = producer_gaps.err().unwrap_or_default();
    if refused_a_buffer {
        // A check verdict *is* obtained here, and its own diagnostics are
        // thrown away rather than reported. Both halves are deliberate.
        //
        // Discarding them is the #1115 guarantee: `reject_memoryview_read`
        // refuses almost every use of a `memoryview`-typed name with
        // `C0001`, so a body that reads its own parameter fails the check,
        // and reporting that verdict here would replace the signature-level
        // `I0405` this early return exists to deliver.
        // `issue_1112_ext_memoryview.rs`'s
        // `a_memoryview_parameter_is_refused_with_i0405_even_when_the_body_reads_it`
        // pins that, and is the regression guard for this call.
        //
        // Obtaining it is the producer gate's own semantic validation, which
        // `producer_gaps_the_check_admits` owns and which is no less required
        // on this path than on the `Err` arm below: a producer whose length the
        // checker refuses (`a = ndarray("x")`) is not an allocation, and
        // `I0405` would prescribe `--ext` for a program `--ext` refuses with
        // `T0033`. The signature gap beside it does not make that remedy any
        // less misdirected.
        //
        // The filter is function-scoped, so a function that both reads its
        // own `memoryview` parameter and allocates loses its (correct)
        // producer gap here. That over-suppression is accepted: the program
        // stays refused, the surviving `I0405` still prescribes `--ext`, and
        // `--ext` genuinely fixes it, so no user is misdirected -- they see
        // one fewer redundant line. The alternative, excluding `C0001` from
        // the filtering verdict by diagnostic code, was rejected: it would
        // encode "C0001 is the set of refusals `--ext` removes" in a fourth
        // place no canonical enumeration owns, which is the treadmill
        // https://github.com/rotnov/pycc/issues/1168 was filed to stop.
        let producer_gaps = match pycc_types::check_and_resolve_all_keyed(&hir) {
            Ok(_) => producer_gaps,
            Err(check_keyed) => {
                crate::memoryview_mode::producer_gaps_the_check_admits(producer_gaps, &check_keyed)
            }
        };
        keyed.extend(
            producer_gaps
                .into_iter()
                .map(|(index, diagnostic)| (sources.owner_of_item(index), diagnostic)),
        );
        return Err(sources.group(keyed));
    }
    // The producer walk's own semantic validation, and the one place it
    // happens: `producer_gaps_the_check_admits` owns why the join is here,
    // why it is per function, and why the gate cannot validate a length
    // itself. A surviving gap is reported *beside* the type errors rather
    // than swallowed by them, so a clean allocating function is still named
    // when some other function in the same program fails to check.
    let resolved = match pycc_types::check_and_resolve_all_keyed(&hir) {
        Ok(resolved) => resolved,
        Err(check_keyed) => {
            // The import gaps collected above are dropped on this path,
            // exactly as the `?` this `match` replaced dropped them: a
            // program with both a type error and a foreign import reports
            // the type error alone, which `issue_1080_foreign_object.rs`'s
            // `a_type_error_is_reported_before_the_native_foreign_refusal`
            // pins. A producer gap is not an import gap -- it has survived
            // its own per-function filter against this very verdict, so it
            // is reported beside the type errors rather than behind them.
            let mut refused =
                crate::memoryview_mode::producer_gaps_the_check_admits(producer_gaps, &check_keyed)
                    .into_iter()
                    .map(|(index, diagnostic)| (sources.owner_of_item(index), diagnostic))
                    .collect::<Vec<_>>();
            refused.extend(attribute(&sources, check_keyed));
            return Err(sources.group(refused));
        }
    };
    keyed.extend(
        producer_gaps
            .into_iter()
            .map(|(index, diagnostic)| (sources.owner_of_item(index), diagnostic)),
    );
    if keyed.is_empty() {
        return Ok((resolved, needs_interpreter));
    }
    Err(sources.group(keyed))
}

fn attribute(
    sources: &ProgramSources,
    keyed: pycc_types::KeyedDiagnostics,
) -> Vec<(usize, Diagnostic)> {
    keyed
        .into_iter()
        .map(|(key, diagnostic)| (sources.owner(key), diagnostic))
        .collect()
}

/// Renders every collected diagnostic for one file, in order, into one
/// string. Human renders are concatenated with no separator (exactly how
/// `check_paths` already concatenates per-file renders); JSON renders are
/// one object per line (JSON Lines), the shape multi-file `check` already
/// produces. The caller decides the stream (stdout for `check`, stderr for
/// `build`/`run`) and the exit code.
fn render_all(
    diagnostics: &[Diagnostic],
    path: &str,
    source: &str,
    error_format: ErrorFormat,
) -> String {
    let mut out = String::new();
    for diagnostic in diagnostics {
        match error_format {
            ErrorFormat::Human => out.push_str(&pycc_diag::render_human(diagnostic, path, source)),
            ErrorFormat::Json => {
                out.push_str(&pycc_diag::render_json(diagnostic, path, source));
                out.push('\n');
            }
        }
    }
    out
}

fn render_files(files: &[FileDiagnostics], error_format: ErrorFormat) -> String {
    let mut out = String::new();
    for file in files {
        out.push_str(&render_all(
            &file.diagnostics,
            &file.path,
            &file.source,
            error_format,
        ));
    }
    out
}

fn report_input_failure(path: &str, message: &str) -> u8 {
    eprintln!(
        "error: could not read `{}`: {message}",
        pycc_diag::display_path(path)
    );
    2
}

/// `pycc check`'s reporter: diagnostics go to stdout in the selected
/// `--error-format`, every collected one; returns the exit code for this
/// file (`1` for any compile diagnostic, `2` for an unreadable input).
pub(crate) fn report_check_failure(failure: FrontendFailure, error_format: ErrorFormat) -> u8 {
    match failure {
        FrontendFailure::Input { path, message } => report_input_failure(&path, &message),
        FrontendFailure::Compile { files } => {
            print!("{}", render_files(&files, error_format));
            1
        }
    }
}

/// `pycc build`/`pycc run`'s reporter: the same human renders as `check`,
/// every collected diagnostic, written to stderr (these commands have no
/// `--error-format`). The build still stops here, before MIR -- only the
/// reporting changed with #864, not the fail-fast semantics.
pub(crate) fn report_build_failure(failure: FrontendFailure) -> u8 {
    match failure {
        FrontendFailure::Input { path, message } => report_input_failure(&path, &message),
        FrontendFailure::Compile { files } => {
            eprint!("{}", render_files(&files, ErrorFormat::Human));
            1
        }
    }
}
