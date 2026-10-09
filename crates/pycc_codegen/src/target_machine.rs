//! LLVM target-registry initialization and target-machine construction.
//!
//! # Concurrency contract
//!
//! LLVM's target registry is process-global mutable state. `Target::initialize_all`
//! walks every built-in backend and stores its constructor function pointers into
//! that registry: `RegisterTargetMachine`, `RegisterMCAsmInfo`, `RegisterMCInstrInfo`
//! and `RegisterMCSubtargetInfo` in LLVM 22.1.1's `llvm/MC/TargetRegistry.h` are all
//! unconditional field assignments, and the header states that clients are
//! responsible for ensuring registration does not race with access to the registry.
//!
//! inkwell 0.9.0 write-locks its own `TARGET_LOCK` around `initialize_all`, so two
//! concurrent initializations cannot interleave with each other. It does *not* take
//! that lock in `Target::create_target_machine`, which reads the very fields
//! `initialize_all` writes. A second thread calling `create_target_machine` while a
//! first thread is still inside `initialize_all` is therefore a data race in the
//! formal (and sanitizer-detectable) sense.
//!
//! In practice the stores are same-valued -- every initialization writes the same
//! constructor pointers -- so this has never been observed to miscompile anything,
//! and it is not the suspected cause of any known defect. The guard below is what
//! the header's contract asks for, not a fix for an observed failure: `initialize_all`
//! runs exactly once per process, so no thread can ever read the registry while it is
//! being written. This is a precondition for the per-module parallel codegen pipeline
//! `docs/ARCHITECTURE.md` plans.

use inkwell::OptimizationLevel;
use inkwell::targets::{
    CodeModel, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::llvm_string_to_owned;

/// Set exactly once, by the first thread to reach [`target_machine_for`].
static TARGET_INIT: OnceLock<()> = OnceLock::new();

/// Number of times the [`TARGET_INIT`] initializer body has actually run.
///
/// A test-observability hook only: the whole point of the guard is that this
/// never exceeds 1, and that is not observable from `initialize_all` itself.
static TARGET_INIT_COUNT: AtomicUsize = AtomicUsize::new(0);

/// How many times LLVM's target registry has been initialized in this process.
#[cfg(test)]
pub(crate) fn target_init_count() -> usize {
    TARGET_INIT_COUNT.load(Ordering::SeqCst)
}

/// Builds the [`TargetMachine`] for `target_triple` (the host's own triple when
/// `None`), initializing LLVM's target registry first if no thread has yet.
///
/// Safe to call concurrently from any number of threads.
pub(crate) fn target_machine_for(
    target_triple: Option<&str>,
    release: bool,
) -> Result<TargetMachine, String> {
    TARGET_INIT.get_or_init(|| {
        TARGET_INIT_COUNT.fetch_add(1, Ordering::SeqCst);
        // initialize_all (not initialize_native): a requested target_triple may
        // not match the host's own architecture, and LLVM only has codegen
        // support for a target's backend if that backend was initialized.
        Target::initialize_all(&InitializationConfig::default());
    });
    // ManuallyDrop, not a plain value: see D-029. TargetTriple wraps an
    // LLVMString (inkwell's own message wrapper around LLVMCreateMessage /
    // LLVMGetDefaultTargetTriple), whose Drop calls LLVMDisposeMessage --
    // this crashes on Windows against the official prebuilt LLVM 22.1.1
    // release. Suppressing the drop here, at the point of creation, covers
    // every exit path uniformly (the early `?` below included), not just
    // the success path a trailing forget would. Leaks one small string per
    // compile on every platform -- negligible in a short-lived CLI process,
    // and simpler than cfg-gating a type difference for a Windows-only leak.
    let triple = std::mem::ManuallyDrop::new(match target_triple {
        Some(t) => TargetTriple::create(t),
        None => TargetMachine::get_default_triple(),
    });
    let target = Target::from_triple(&triple).map_err(|e| {
        format!(
            "pycc_codegen: `{}` is not a target LLVM knows how to generate code for: {}",
            triple.as_str().to_string_lossy(),
            llvm_string_to_owned(e)
        )
    })?;
    Ok(target
        .create_target_machine(
            &triple,
            "generic",
            "",
            if release {
                OptimizationLevel::Aggressive
            } else {
                OptimizationLevel::None
            },
            // `RelocMode::Default` resolves to absolute (non-PIC)
            // addressing for this LLVM/target pairing on Linux, but
            // Ubuntu's `cc`/`gcc` links as a PIE by default (D-073):
            // large-`.rodata` programs (confirmed with the
            // `mandelbrot_ascii` fixture -- its ASCII palette/float
            // constants push a relocation past what a 32-bit absolute
            // reloc can express in a PIE) fail with "relocation
            // R_X86_64_32 against `.rodata' can not be used when making
            // a PIE object". `RelocMode::PIC` matches every Tier-1
            // linker's actual default (mandatory on macOS, standard on
            // Windows/MSVC, and Linux's own PIE default) uniformly.
            RelocMode::PIC,
            CodeModel::Default,
        )
        .expect(
            "creating a target machine with generic CPU/features should never fail for a \
             triple Target::from_triple has already accepted",
        ))
}
