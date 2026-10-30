//! The `--ext`-specific planning of `try_build` (split out of
//! `src/build_pipeline.rs` when the whole-package build added its member
//! builds): the output contract, the export set and the generated C sources
//! of one module's artifact.

use crate::frontend::{self, report_build_failure};
use crate::{ext_build, ext_output, memoryview_mode};
use std::path::Path;
use std::process::ExitCode;

/// Everything `try_build`'s link step needs that is specific to `--ext`.
///
/// `artifact` replaces `OUT` as the linker's `-o`: `ext_output::resolve`
/// may have appended the platform's stable-ABI suffix to it, and CPython's
/// finder will only import a file whose name it recognizes.
#[derive(Debug)]
pub(super) struct ExtPlan {
    pub(super) artifact: std::path::PathBuf,
    pub(super) compile_args: Vec<std::ffi::OsString>,
    pub(super) link_args: Vec<std::ffi::OsString>,
    /// The module-level functions the module publishes (a package member's
    /// interface lists some of them).
    pub(super) published: std::collections::BTreeSet<String>,
    /// The classes whose instances cross into CPython as objects of their
    /// own type (a package member's interface lists some of them).
    pub(super) carriers: std::collections::BTreeSet<String>,
    /// v0.10: each constructor whose host call binds keywords, and the
    /// names it binds (a package member's interface lists them).
    pub(super) init_names: std::collections::BTreeMap<String, Vec<String>>,
}

/// Resolves `OUT` against the `--ext` output contract, reporting a rejected
/// path exactly as `plan_ext` does (`src/ext_output.rs` owns the messages).
///
/// Exists because `try_build` needs the resolved module name *before* the
/// frontend runs -- it is the entry module's `__name__` (#1156) -- while
/// `plan_ext` needs the resolved artifact path after it. `ext_output::resolve`
/// is pure, so calling it from both places costs nothing and keeps
/// `plan_ext`'s own failure ordering intact.
pub(super) fn resolve_ext_output(
    out: &Path,
    target: Option<&str>,
) -> Result<ext_output::ExtOutput, ExitCode> {
    ext_output::resolve(
        out,
        &ext_build::ExtLinkPlatform::resolve(target).suffix_platform(),
    )
    .map_err(|e| {
        eprintln!("error: {}", e.message());
        ExitCode::from(2)
    })
}

/// Resolves the `--ext` output, export set and host toolchain, and writes
/// the two C files the link step compiles alongside the emitted object.
///
/// Ordered failure-cheapest-first, and every failure here happens before
/// codegen: the output contract and the `C0003` export gaps are properties
/// of the invocation and the program, the header probe is a property of the
/// host, and none of them becomes more informative for having run LLVM.
///
/// The C files are written next to `obj_path`, which the caller already
/// owns as a `pycc_scratch::ScratchDir` whose `Drop` removes the whole
/// directory on every exit path (#783). Nothing here writes beside the
/// user's `-o`, which stays the single persistent output.
pub(super) fn plan_ext(
    source_path: &Path,
    out: &Path,
    target: Option<&str>,
    typed_hir: &pycc_hir::HirModule,
    toolchain: &ext_build::ExtToolchain,
    obj_path: &Path,
) -> Result<ExtPlan, ExitCode> {
    let platform = ext_build::ExtLinkPlatform::resolve(target);
    // A package member's sibling classes are another module's: it neither
    // publishes them nor gives them a CPython type (`crate::package_build`).
    let without_stubs = crate::package_build::without_sibling_classes(typed_hir);
    let typed_hir = without_stubs.as_ref().unwrap_or(typed_hir);
    // A package member (`crate::package_build`) has no artifact of its own:
    // its module name is its dotted name, and the package build links it.
    let member = crate::package_build::member();
    let output = match member {
        Some(member) => ext_output::ExtOutput {
            module_name: member.dotted.clone(),
            artifact: out.to_path_buf(),
        },
        None => ext_output::resolve(out, &platform.suffix_platform()).map_err(|e| {
            eprintln!("error: {}", e.message());
            ExitCode::from(2)
        })?,
    };
    // #1461 and #1467 both read the entry module's own source: keyword
    // binding needs the `def`s HIR no longer spells, and the PEP 562 hooks
    // to publish are the entry module's own (see `ext_build::module_hooks`).
    let entry_source = crate::fallback::entry_source(source_path);
    // A located `C0001`, so it is rendered against the real source text.
    let hooks = ext_build::EntryHooks::from_source(&entry_source).map_err(|refusals| {
        ExitCode::from(report_build_failure(frontend::FrontendFailure::compile(
            &source_path.display().to_string(),
            &entry_source,
            refusals,
        )))
    })?;
    let mut exports =
        ext_build::collect_exports_with_hooks(typed_hir, hooks.defined()).map_err(|gaps| {
            // Span-less `C0003`s (a lowered `HirItem::Function` carries no
            // source range), so the empty source text below is never read:
            // `pycc_diag::render_human` renders a span-less diagnostic as
            // exactly `error[C0003]: <message>`.
            ExitCode::from(report_build_failure(frontend::FrontendFailure::compile(
                &source_path.display().to_string(),
                "",
                gaps,
            )))
        })?;
    // Every function whose return type is a buffer except the shapes the
    // `--ext` boundary admits: Part 2b of #1142 (#1164) admitted a public
    // *module-level* export, and #1174 widened that to the whole export
    // set, a public method of a public class included. What is left --
    // a private function, a private method, a method of a private or
    // exception class, a specialization -- would otherwise reach codegen's
    // own panic for a `memoryview`-typed call result;
    // `refuse_in_ext_mode`'s own doc comment carries the argument in full,
    // including why #1174's interception at the four method-resolution
    // exits is what makes the widening safe. It is handed the export set
    // because a buffer return type is carriable now, so `collect_exports`
    // above no longer refuses one on its own.
    //
    // Position: this runs inside `plan_ext`, which `run_build` calls
    // *before* `compile_to_object_with_options`, so a refusal here is what
    // keeps that panic unreachable rather than merely unlikely.
    memoryview_mode::refuse_in_ext_mode(typed_hir, &exports).map_err(|gaps| {
        ExitCode::from(report_build_failure(frontend::FrontendFailure::compile(
            &source_path.display().to_string(),
            "",
            gaps,
        )))
    })?;
    let carriers = ext_build::collect_carrier_classes(typed_hir);
    // #1427: a comparison or hash dunder the artifact cannot install is a
    // span-less `C0003`, reported exactly as an export gap is,
    // and before the interpreter probe, so the refusal needs no CPython.
    let slot_gaps =
        ext_build::collect_slot_dunders(typed_hir, &carriers, ext_build::SlotArtifact::Ext);
    let slots = slot_gaps.map_err(|gaps| {
        let failure =
            frontend::FrontendFailure::compile(&source_path.display().to_string(), "", gaps);
        ExitCode::from(report_build_failure(failure))
    })?;
    let probe = toolchain.probe().map_err(|e| {
        eprintln!("error: {e}");
        ExitCode::from(2)
    })?;
    // Rendered into a binding first so the call below fits one line: a
    // multi-line `foo(\n  ..\n)?;` puts the `?`'s early-return arm on a
    // line of its own, which no passing build ever executes and which
    // `scripts/check_diff_coverage.py` then reports as an uncovered
    // changed line (D-242 rule 1).
    let classes = ext_build::collect_user_exception_classes(typed_hir);
    // #1461: which exports a host call may name keywords for is read from
    // the entry source itself (see `ext_build::keywords`), and bound before
    // `collect_class_publications` copies the method exports.
    let mut signatures = ext_build::SourceSignatures::from_source(&entry_source);
    // v0.10: the sibling classes standing for another module's (a package
    // member's): a subclass inheriting one's constructor binds keywords
    // as the other module's does.
    signatures.extend(ext_build::SourceSignatures::from_source(
        &crate::package_build::stub_sources(),
    ));
    ext_build::bind_keyword_names(typed_hir, &signatures, &mut exports);
    let mut publications = ext_build::collect_class_publications(typed_hir, &exports);
    // v0.9: each published class is made with its statement's real bases.
    ext_build::attach_class_bases(&mut publications, typed_hir, &entry_source);
    let mut ctors = ext_build::collect_constructors(typed_hir, &publications);
    ext_build::bind_ctor_keyword_names(typed_hir, &signatures, &mut ctors);
    // #1448: every carrier type's descriptor table, published or not.
    let getsets = ext_build::collect_carrier_getsets(typed_hir, &carriers);
    let inc_body = ext_build::generate_exports_inc_with_slots(
        &output.module_name,
        &exports,
        &classes,
        &publications,
        &ctors,
        &carriers,
        &getsets,
        &slots,
    );
    // The other modules' conversions its wrappers call for their classes.
    let inc_body = format!(
        "{}{inc_body}",
        crate::package_build::sibling_c_declarations()
    );
    // #1194: the Python wrappers of the signatures the boundary cannot bind
    // alone, planned from the entry source exactly as the loader planned
    // their compiled entries (`pycc_ast::split_ext_signatures`).
    let published_signatures = pycc_parser::parse_all(&entry_source)
        .map(|parsed| {
            pycc_ast::ext_signatures(
                &parsed,
                &entry_source,
                crate::mode::non_strict_for(source_path),
            )
        })
        .unwrap_or_default();
    // The fallback rewrite's CPython source and the signature wrappers'
    // (after it), compiled by the target interpreter now instead of by
    // `compile()` at every import.
    let mut chunks = crate::fallback::code_chunks();
    let first_wrapper = chunks.len();
    chunks.extend(published_signatures.iter().map(ext_build::signature_chunk));
    let inc_body = ext_build::install_signatures(inc_body, &published_signatures, first_wrapper);
    let compiled = toolchain
        .precompile_interpreter()
        .filter(|_| !chunks.is_empty())
        .and_then(|python| ext_build::precompile(python, &chunks));
    let inc_body = ext_build::install_precompiled(inc_body, &chunks, compiled.as_ref());
    let inc_body = ext_build::with_module_doc(inc_body, &entry_source);
    let sources = ext_build::shim_sources(obj_path, &inc_body);
    for (path, text) in &sources {
        write_ext_source(path, text)?;
    }
    let map = obj_path.with_file_name(ext_build::EXPORT_MAP_NAME);
    let mut link_args = ext_build::ext_link_args(platform, &probe.libs);
    // The link decides whether to pack relocations (`try_build`); the probe
    // that decides it runs now, beside codegen. A package member never
    // links (the package's one link packs), so it does not probe.
    if member.is_none() {
        ext_build::start_linker_probe(target);
    }
    let export_args = ext_build::ext_export_args(platform, &map, false);
    if !export_args.is_empty() {
        write_ext_source(&map, &ext_build::export_map())?;
        link_args.extend(export_args);
    }
    let mut compile_args = ext_build::ext_compile_args(platform, &probe.include, &sources[0].0);
    match member {
        // A member compiles its own unit only, after its symbol names; the
        // package build compiles the runtime unit once for all of them.
        Some(member) => {
            let header = obj_path.with_file_name("pycc_ext_member.h");
            write_ext_source(
                &header,
                &crate::package_build::member_header(&member.prefix, &inc_body),
            )?;
            compile_args.insert(0, std::ffi::OsString::from("-DPYCC_EXT_PACKAGE_MEMBER"));
        }
        None => compile_args.push(sources[2].0.clone().into_os_string()),
    }
    let published = exports
        .iter()
        .filter(|export| export.class.is_none())
        .map(|export| export.name.clone())
        .collect();
    Ok(ExtPlan {
        compile_args,
        link_args,
        artifact: output.artifact,
        published,
        carriers: carriers.iter().map(|carrier| carrier.class.clone()).collect(),
        init_names: ctors
            .iter()
            .filter_map(|ctor| Some((ctor.name.clone(), ctor.keyword_names.clone()?)))
            .collect(),
    })
}

/// Writes one generated C file into the caller-owned scratch directory.
/// A failure here is an environment failure (an unwritable scratch), not a
/// pycc invariant -- reported at exit 2 like `create_scratch`'s own.
pub(super) fn write_ext_source(path: &Path, contents: &str) -> Result<(), ExitCode> {
    std::fs::write(path, contents).map_err(|e| {
        eprintln!(
            "error: could not write the --ext build source `{}`: {e}",
            pycc_diag::display_path(&path.to_string_lossy())
        );
        ExitCode::from(2)
    })
}
