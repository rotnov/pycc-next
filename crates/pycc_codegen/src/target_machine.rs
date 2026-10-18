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
        set_backend_options();
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
    let deployment = if target_triple.is_none() && cfg!(target_os = "macos") {
        macos_deployment_triple(
            std::env::consts::ARCH,
            std::env::var("MACOSX_DEPLOYMENT_TARGET").ok().as_deref(),
        )
    } else {
        None
    };
    let triple = std::mem::ManuallyDrop::new(match (target_triple, deployment.as_deref()) {
        (Some(t), _) | (None, Some(t)) => TargetTriple::create(t),
        (None, None) => TargetMachine::get_default_triple(),
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

/// The LLVM backend options every object pycc emits is built with, set once
/// per process before the first target machine exists (`cl::opt`s are
/// process-global and `LLVMParseCommandLineOptions` may run only once).
///
/// `-x86-branches-within-32B-boundaries`: the x86 assembler pads so that no
/// jump crosses or ends on a 32-byte boundary. Skylake-derived Intel cores
/// (Skylake through Cascade Lake, including the Xeons of shared CI and cloud
/// machines) run a microcode update for erratum SKX102 that keeps such a
/// jump's code window out of the decoded-instruction cache, so a hot loop
/// whose branch lands on a boundary runs measurably slower. Without the
/// padding, where a loop lands depends on the size of every function placed
/// before it: diff-match-patch's `diff_bisect` became 7-13% slower between
/// two commits whose code for it was identical, instruction for instruction,
/// because unrelated functions grew (v0.5, `CHANGELOG.md`). With it,
/// the hot loops run as fast at every placement as at the best one.
/// The padding is `nop`s, not instruction prefixes
/// (`-x86-pad-max-prefix-size=5`, used until v0.8): prefix padding doubled
/// the time LLVM takes to emit an object (the assembler's relaxation
/// re-lays out every padded fragment), which pushed the mistune and
/// diff-match-patch builds past CI's 10 s package build bound, for about 1%
/// fewer executed instructions (diff-match-patch's `diff_main`: 130.1M
/// against 131.4M). The option belongs to the x86 backend and changes
/// nothing for other targets.
fn set_backend_options() {
    let args: [&std::ffi::CStr; 2] = [c"pycc", c"-x86-branches-within-32B-boundaries"];
    let argv: Vec<*const std::ffi::c_char> = args.iter().map(|arg| arg.as_ptr()).collect();
    // SAFETY: `argv` holds `args.len()` pointers to NUL-terminated strings
    // that outlive the call, and `TARGET_INIT` runs this once per process,
    // before any target machine reads the options. Both options are
    // registered by the x86 backend `initialize_all` linked in.
    unsafe {
        inkwell::llvm_sys::support::LLVMParseCommandLineOptions(
            argv.len() as i32,
            argv.as_ptr(),
            std::ptr::null(),
        );
    }
}

/// The host triple for a native macOS build pinned to the deployment target
/// `MACOSX_DEPLOYMENT_TARGET` names (`arm64-apple-macosx26.0`), so the
/// object file's minimum OS matches what the `cc` link -- which reads the
/// same variable -- and `rustc`'s `libpycc_rt.a` use. `None` when the
/// variable is unset or not a dotted version: LLVM's default triple then
/// targets the running macOS, as `cc` does by default.
fn macos_deployment_triple(arch: &str, deployment_target: Option<&str>) -> Option<String> {
    let version = deployment_target?.trim();
    let valid = !version.is_empty()
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
    if !valid {
        return None;
    }
    let arch = if arch == "aarch64" { "arm64" } else { arch };
    Some(format!("{arch}-apple-macosx{version}"))
}

#[cfg(test)]
mod deployment_tests {
    use super::macos_deployment_triple;

    #[test]
    fn a_deployment_target_pins_the_macos_triple() {
        assert_eq!(
            macos_deployment_triple("aarch64", Some("26.0")).as_deref(),
            Some("arm64-apple-macosx26.0")
        );
        assert_eq!(
            macos_deployment_triple("x86_64", Some(" 27 ")).as_deref(),
            Some("x86_64-apple-macosx27")
        );
    }

    #[test]
    fn an_unset_or_malformed_deployment_target_keeps_the_host_default() {
        for bad in [None, Some(""), Some("26."), Some("v26"), Some("26.0;x")] {
            assert_eq!(macos_deployment_triple("aarch64", bad), None, "{bad:?}");
        }
    }
}
