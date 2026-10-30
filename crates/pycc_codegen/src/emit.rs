//! The end of the compile driver: the optimization pipeline and object
//! emission, split out of `compile.rs` when emission learned to run LLVM's
//! backend on several threads (v0.5 accept 4, `crate::split_emit`).

use super::*;
use std::path::PathBuf;

/// Optimizes the verified `module` (`--release`) and writes it as one
/// object at `output_path`, or, with `split`, as several objects when the
/// module is large enough for a parallel backend to pay
/// (`crate::split_emit`). Returns every object written, `output_path`
/// first.
pub(crate) fn optimize_and_emit(
    module: &inkwell::module::Module<'_>,
    mir: &MirModule,
    options: &CompileOptions,
    mut observer: Option<&mut CodegenObserver<'_>>,
    output_path: &Path,
    split: bool,
) -> Result<Vec<PathBuf>, String> {
    let target_triple = options.target_triple.as_deref();
    let release = options.release;
    // See `target_machine::target_machine_for` for the LLVM target-registry
    // initialization contract this call satisfies, and for the D-029 and
    // D-073 reasoning behind the triple and relocation-model choices.
    let target_machine = target_machine::target_machine_for(target_triple, release)?;
    crate::store_width::check(module, &target_machine.get_target_data());
    // `--release`'s whole-module optimization pipeline (D-094). This is
    // "maximum whole-module optimization," not literal cross-translation-
    // unit LTO: pycc emits exactly one LLVM module per compilation today
    // (single-file only until v0.4's multi-file support), so there is only
    // ever one module for `"default<O3>"` to optimize.
    //
    // Deliberately `.expect(..)`, not `.map_err(llvm_string_to_owned)?` like
    // `Target::from_triple` (in `target_machine`)/`write_to_file` below:
    // those two are genuine,
    // externally-triggerable failure modes with their own dedicated tests
    // and reachable `Err` paths this crate's 100% region-coverage gate
    // (D-014) can actually exercise. `run_passes` here runs a fixed,
    // always-valid pipeline string against a module `verify_module` has
    // already accepted (skipped only on Windows per D-029's own "no
    // Windows-specific IR-building path exists" reasoning, which applies
    // here too) -- there is no way to make this fail that a test could
    // construct, so this stays infallible given how this function always
    // calls it, the same treatment `create_target_machine` gets in
    // `target_machine` and `module.verify()` gets in `verify_module` below.
    // On the
    // vanishingly narrow chance this ever legitimately panics on Windows,
    // the `LLVMString`'s `Drop` would run during unwinding (D-029's crash)
    // -- an accepted, currently unreachable risk, not a silently ignored one.
    // A generator is an LLVM coroutine, which only LLVM's coroutine passes
    // lower: a debug build of a module with one runs the `O0` pipeline,
    // which includes them.
    let has_coroutines = mir.items.iter().any(|item| {
        matches!(
            item,
            MirItem::Function {
                return_ty: pycc_mir::Ty::Generator(_),
                ..
            }
        )
    });
    if !release && has_coroutines {
        module
            .run_passes("default<O0>", &target_machine, PassBuilderOptions::create())
            .expect("LLVM's \"default<O0>\" pipeline lowers a verified module's coroutines");
    }
    // Developer hook for performance work: `PYCC_DUMP_LLVM=<path>` writes the
    // module's IR before (`<path>.pre.ll`) and after (`<path>`) the pipeline.
    let dump_path = std::env::var_os("PYCC_DUMP_LLVM").map(std::path::PathBuf::from);
    if let Some(path) = &dump_path {
        let mut pre = path.clone().into_os_string();
        pre.push(".pre.ll");
        let _ = module.print_to_file(std::path::Path::new(&pre));
    }
    // `--release` counts CPython objects inline (`crate::object_rc`).
    if release {
        crate::object_rc::inline_object_rc(module, &target_machine.get_target_data());
    }
    let optimizing = std::time::Instant::now();
    let applied_pipeline = if release {
        module
            .run_passes(
                RELEASE_PASS_PIPELINE,
                &target_machine,
                PassBuilderOptions::create(),
            )
            .expect(
                "LLVM's \"default<O3>\" pipeline should never fail against a module this \
                 function has already verified, using a fixed, always-valid pipeline string",
            );
        Some(RELEASE_PASS_PIPELINE)
    } else {
        None
    };
    if let Some(path) = &dump_path {
        let _ = module.print_to_file(path);
    }
    if let Some(observer) = observer.as_mut() {
        observer(module, applied_pipeline);
    }
    report_time("llvm: optimize", optimizing);
    // A package member's symbols, named per module before the backend
    // (and before the split promotes its local symbols).
    let member_prefix = options.symbol_prefix.as_deref().unwrap_or("");
    if !member_prefix.is_empty() {
        crate::member_symbols::rename(module, member_prefix);
    }
    // A section per function on ELF, so the assembler lays out each
    // function alone (`crate::function_sections`).
    let triple = target_machine.get_triple();
    crate::function_sections::assign(module, &triple.as_str().to_string_lossy());
    let emitting = std::time::Instant::now();
    let objects = if split && let Some(parts) = crate::split_emit::plan(module, member_prefix) {
        crate::split_emit::emit(module, &parts, output_path, target_triple, release)?
    } else {
        target_machine
            .write_to_file(module, FileType::Object, output_path)
            .map_err(llvm_string_to_owned)?;
        vec![output_path.to_path_buf()]
    };
    report_time(
        if objects.len() > 1 {
            "llvm: emit (parallel)"
        } else {
            "llvm: emit"
        },
        emitting,
    );
    Ok(objects)
}

/// `PYCC_TIME=1` (the driver's phase report): how long one LLVM phase
/// took, in the driver's report format.
fn report_time(phase: &str, started: std::time::Instant) {
    if std::env::var_os("PYCC_TIME").is_some_and(|v| !v.is_empty() && v != "0") {
        eprintln!(
            "pycc-time: {phase:<28} {:>9.3}s  x1",
            started.elapsed().as_secs_f64()
        );
    }
}
