//! The `--ext` build seam: what a hosted CPython extension artifact needs
//! that a native executable does not (D-244, #1036, Part 1 of #1025).
//!
//! `src/ext_output.rs` already owns *where* the artifact is written and what
//! its module is called. This module owns everything else `try_build`'s ext
//! branch needs:
//!
//! * the CPython toolchain probe (interpreter, header directory, stable-ABI
//!   floor), [`ExtToolchain`];
//! * the per-platform shared-object link argv, [`ExtLinkPlatform`] and
//!   [`ext_link_args`];
//! * the export set and its `C0003` capability gaps, [`collect_exports_with_hooks`];
//! * the generated C companion to the fixed shim, [`generate_exports_inc_with_slots`]
//!   and [`SHIM_C`].
//!
//! Every item here except [`ExtToolchain::probe`] is a pure function of its
//! arguments. That is deliberate, and it is the same property
//! `src/ext_output.rs` documents for its own contract: the three link arms
//! and both header-resolution failures must be executable from an ordinary
//! unit test on one host, because `.github/workflows/ci.yml`'s coverage job
//! runs `llvm-cov` without `--include-ignored` -- an `#[ignore]`d
//! CPython-driven test contributes exactly zero coverage while its lines
//! stay in `scripts/check_diff_coverage.py`'s denominator. A `cfg!(windows)`
//! branch would be just as unexecutable as a `#[cfg]` one is *coverable*,
//! so the platform arms are selected from the resolved **target triple**,
//! never from the building host -- which is also what makes
//! `pycc build --target x86_64-pc-windows-msvc --ext` emit a Windows argv
//! from a macOS host at all.

use crate::ext_output::ExtPlatform;
use pycc_diag::Diagnostic;
use pycc_hir::{
    BUILTIN_EXCEPTION_CLASSES, FIRST_USER_EXCEPTION_TYPE_TAG, HirClassDef, HirItem, HirModule, Ty,
    flat_attr_layout, is_builtin_exception_class, is_public_name,
};
use std::collections::{BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The fixed, hand-written half of every `ext` artifact, embedded at compile
/// time. See `src/ext/pycc_ext_module.c`'s own header comment for why it is
/// C at all and why it is embedded rather than located by path.
pub(crate) const SHIM_C: &str = include_str!("ext/pycc_ext_module.c");

/// The shim's file name in the build scratch directory. The generated
/// companion is [`EXPORTS_INC_NAME`], which the shim `#include`s by this
/// exact spelling, so the two must be written side by side.
pub(crate) const SHIM_C_NAME: &str = "pycc_ext_module.c";

/// The generated companion's file name -- `#include`d by [`SHIM_C`], never
/// compiled on its own.
pub(crate) const EXPORTS_INC_NAME: &str = "pycc_ext_exports.inc";

/// The stable-ABI floor: `Py_LIMITED_API 0x030D0000` in the shim pins the
/// artifact to CPython 3.13's limited API, so building against an older
/// interpreter's headers would compile against a smaller stable ABI than the
/// one the shim declares. 3.13 is also the first release whose
/// `EXTENSION_SUFFIXES` the `ext_output` contract's `.abi3.so` spelling is
/// derived from.
pub(crate) const MIN_PYTHON: (u32, u32) = (3, 13);

/// `C0003`: an `ext`-mode capability gap.
///
/// Distinct from `C0001` ("construct not supported yet") because the
/// construct here *is* supported -- `pycc` compiles the function perfectly
/// well for `native` mode. What is missing is the `PyObject*` boundary for
/// its signature; `docs/RUNTIME.md`'s admissibility matrix is the canonical
/// statement of which types that boundary carries, in which direction, and
/// what each admitted one narrows. `docs/DIAGNOSTICS.md` carries the
/// registry entry and `crates/pycc_diag/src/explain.rs` the long-form
/// explanation.
pub(crate) const EXT_CAPABILITY_CODE: &str = "C0003";

/// What a CPython installation told us about itself.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExtProbe {
    /// `(major, minor)`, checked against [`MIN_PYTHON`].
    pub(crate) version: (u32, u32),
    /// The directory holding `Python.h`, passed to the compiler as `-I`.
    pub(crate) include: PathBuf,
    /// Where an import library lives. Only Windows links against one; every
    /// other platform resolves the interpreter's symbols at load time and
    /// must *not* be given a `-lpython`, which is what would pin the
    /// artifact to one interpreter build.
    pub(crate) libs: PathBuf,
}

/// The CPython installation an `ext` build compiles against.
///
/// Constructed from the environment in production ([`ExtToolchain::from_env`])
/// and directly in tests (`ExtToolchain::with_probe`) -- never by a test
/// setting a process-wide environment variable, which races every other test
/// in the same binary.
#[derive(Debug, Clone)]
pub(crate) struct ExtToolchain {
    interpreter: OsString,
    probe_override: Option<ExtProbe>,
}

/// Asks an interpreter the three things an `ext` build needs, one per line.
/// Kept as a single expression-free script so no shell quoting is involved:
/// it is passed as one `-c` argument to the interpreter process directly.
const PROBE_SCRIPT: &str = "import os,sys,sysconfig\n\
                            print('%d.%d' % sys.version_info[:2])\n\
                            print(sysconfig.get_path('include'))\n\
                            print(os.path.join(sys.base_prefix, 'libs'))\n";

impl ExtToolchain {
    /// The production constructor. `PYCC_PYTHON` names the interpreter to
    /// probe (default `python3`); `PYCC_PYTHON_INCLUDE`, when set, supplies
    /// the header directory without probing at all, for a cross build or a
    /// sysroot whose interpreter cannot run on this host.
    ///
    /// `PYCC_PYTHON_INCLUDE` records [`MIN_PYTHON`] as the version, which is
    /// an *assertion by the caller*, not a measurement: no interpreter runs,
    /// so nothing here can read the headers' real version, and
    /// [`Self::probe`]'s floor check is therefore satisfied by construction
    /// on this path. Setting it says "these headers are at least the
    /// stable-ABI floor"; if they are not, the failure surfaces in `cc`
    /// against the real `Python.h` rather than here.
    pub(crate) fn from_env() -> Self {
        let interpreter =
            std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| OsString::from("python3"));
        let probe_override = std::env::var_os("PYCC_PYTHON_INCLUDE").map(|include| {
            let include = PathBuf::from(include);
            ExtProbe {
                version: MIN_PYTHON,
                libs: include.with_file_name("libs"),
                include,
            }
        });
        Self {
            interpreter,
            probe_override,
        }
    }

    /// Builds a toolchain that answers from `probe` instead of running an
    /// interpreter. `interpreter` is still recorded, so a caller can prove
    /// the override is what suppressed the spawn.
    #[cfg(test)]
    pub(crate) fn with_probe(interpreter: impl Into<OsString>, probe: ExtProbe) -> Self {
        Self {
            interpreter: interpreter.into(),
            probe_override: Some(probe),
        }
    }

    /// Builds a toolchain that really probes `interpreter`.
    #[cfg(test)]
    pub(crate) fn with_interpreter(interpreter: impl Into<OsString>) -> Self {
        Self {
            interpreter: interpreter.into(),
            probe_override: None,
        }
    }

    /// Resolves the headers to compile against, or an environment-class
    /// message for `try_build` to report at exit 2.
    ///
    /// The directory check runs on an override exactly as it does on a real
    /// probe, and so does the floor check -- but see [`Self::from_env`]: an
    /// override built from `PYCC_PYTHON_INCLUDE` carries [`MIN_PYTHON`] as
    /// its asserted version, so the floor is a real gate only for a probe
    /// that actually ran (or a test-supplied one).
    ///
    /// The directory existing is not enough: `Python.h` itself must be in it.
    /// An interpreter installed without its development package reports a
    /// header directory that is absent or empty, and letting that through
    /// would surface the failure inside the C compiler instead -- which
    /// `docs/CLI_SPEC.md` classifies as a compile error at exit 1, when a
    /// broken CPython development environment is an environment failure at
    /// exit 2.
    pub(crate) fn probe(&self) -> Result<ExtProbe, String> {
        let probe = match &self.probe_override {
            Some(probe) => probe.clone(),
            None => self.run_probe()?,
        };
        check_floor(probe.version)?;
        if !probe.include.is_dir() {
            return Err(format!(
                "CPython header directory `{}` does not exist; --ext compiles against \
                 `Python.h` from the interpreter named by PYCC_PYTHON (default `python3`), \
                 or from PYCC_PYTHON_INCLUDE when that is set",
                probe.include.display()
            ));
        }
        if !probe.include.join("Python.h").is_file() {
            return Err(format!(
                "CPython header directory `{}` contains no `Python.h`; --ext compiles \
                 against that header, so an interpreter without its development package \
                 installed cannot serve it -- install that package, or set \
                 PYCC_PYTHON_INCLUDE to the directory that does hold `Python.h`",
                probe.include.display()
            ));
        }
        Ok(probe)
    }

    fn run_probe(&self) -> Result<ExtProbe, String> {
        let output = probe_command(&self.interpreter, PROBE_SCRIPT)
            .output()
            .map_err(|e| {
                format!(
                    "could not run the CPython interpreter `{}`: {e}; --ext needs one to \
                     locate `Python.h` -- set PYCC_PYTHON to name a different interpreter, \
                     or PYCC_PYTHON_INCLUDE to skip the probe entirely",
                    self.interpreter.to_string_lossy()
                )
            })?;
        if !output.status.success() {
            return Err(format!(
                "the CPython interpreter `{}` failed to report its own configuration \
                 (exit {})",
                self.interpreter.to_string_lossy(),
                output.status.code().unwrap_or(-1)
            ));
        }
        parse_probe_output(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
            format!(
                "the CPython interpreter `{}` reported a configuration this build could \
                 not parse",
                self.interpreter.to_string_lossy()
            )
        })
    }
}

/// The `<interpreter> -I -c <script>` command both CPython probes (this
/// one and the embedded mode's) run. `-I` is isolated mode: the current
/// directory, the user site directory and every `PYTHON*` variable stay off
/// the probe's module search path, so a `sysconfig.py` planted in the
/// project being built cannot execute when the probe imports `sysconfig`.
pub(crate) fn probe_command(interpreter: &OsStr, script: &str) -> std::process::Command {
    let mut command = std::process::Command::new(interpreter);
    command.arg("-I").arg("-c").arg(script);
    command
}

/// Parses [`PROBE_SCRIPT`]'s three lines. Pure, so every malformed shape is
/// reachable from a unit test without an interpreter that produces it.
pub(crate) fn parse_probe_output(stdout: &str) -> Option<ExtProbe> {
    let mut lines = stdout.lines();
    let version = parse_version(lines.next()?)?;
    let include = PathBuf::from(lines.next()?.trim());
    let libs = PathBuf::from(lines.next()?.trim());
    if include.as_os_str().is_empty() {
        return None;
    }
    Some(ExtProbe {
        version,
        include,
        libs,
    })
}

/// Parses a `major.minor` version line. Extra components (`3.13.1`) are
/// ignored rather than rejected: the floor only concerns the first two.
pub(crate) fn parse_version(line: &str) -> Option<(u32, u32)> {
    let line = line.trim();
    let (major, rest) = line.split_once('.')?;
    let minor = rest.split('.').next()?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Enforces [`MIN_PYTHON`].
pub(crate) fn check_floor(version: (u32, u32)) -> Result<(), String> {
    if version >= MIN_PYTHON {
        return Ok(());
    }
    Err(format!(
        "--ext needs CPython {}.{} or newer to build against (found {}.{}): the artifact \
         declares `Py_LIMITED_API 0x030D0000`, so an older interpreter's headers describe a \
         smaller stable ABI than the one it uses",
        MIN_PYTHON.0, MIN_PYTHON.1, version.0, version.1
    ))
}

/// The link-time shape of an `ext` artifact, selected from the resolved
/// target triple.
///
/// Distinct from [`ExtPlatform`], which distinguishes only the two
/// *extension-suffix* families (POSIX vs Windows) because that is all
/// `EXTENSION_SUFFIXES` distinguishes. Linking needs three arms: macOS's
/// two-level namespace makes a `-shared` `.so` with unresolved `Py*` symbols
/// a link error, so it needs a bundle with deferred lookup instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtLinkPlatform {
    /// macOS: a `-bundle` with `-undefined dynamic_lookup`.
    MacOs,
    /// Linux and every other ELF host: a plain `-shared`.
    Linux,
    /// Windows: a DLL that must resolve `Py*` at link time against
    /// `python3.lib`, the stable-ABI import library.
    Windows,
}

impl ExtLinkPlatform {
    /// This build host, for the no-`--target` case. The only `cfg` in this
    /// module: it selects a *value*, so every arm's own behaviour stays
    /// reachable from a unit test on any host.
    pub(crate) const HOST: Self = if cfg!(windows) {
        Self::Windows
    } else if cfg!(target_os = "macos") {
        Self::MacOs
    } else {
        Self::Linux
    };

    /// Classifies a target triple. Unknown triples fall to [`Self::Linux`],
    /// the ELF default -- the same POSIX-default convention
    /// [`crate::ext_output`]'s suffix classification uses, so a triple this
    /// function has never seen gets a plain `-shared` link and a `.so`
    /// rather than a build that refuses to start.
    pub(crate) fn from_target_triple(triple: &str) -> Self {
        if triple.contains("windows") {
            Self::Windows
        } else if triple.contains("apple") || triple.contains("darwin") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }

    /// Resolves the platform for `target`, honouring the same
    /// "target, never host" rule `ext_output` states.
    pub(crate) fn resolve(target: Option<&str>) -> Self {
        target.map_or(Self::HOST, Self::from_target_triple)
    }

    /// The suffix family this link platform writes into.
    pub(crate) fn suffix_platform(self) -> ExtPlatform {
        match self {
            Self::Windows => ExtPlatform::Windows,
            Self::MacOs | Self::Linux => ExtPlatform::Unix,
        }
    }
}

/// The platform-specific arguments that turn a link into an importable
/// extension module, in the order the driver receives them.
///
/// Deliberately returns `OsString`s built from `Path::join`, never a
/// formatted string: a rendered `Path` carries the *building host's*
/// separator, so a `-L` argument assembled by `format!` would spell a
/// Windows target's library directory with `/` when cross-building from
/// macOS. (PR 1a's only CI failure was a test asserting a rendered path.)
///
/// No position-independent-code work is needed on the *Rust* side to pull
/// `libpycc_rt.a` into a shared object: none of the five Tier-1 target
/// specs sets `relocation-model`, so all five take `rustc`'s own default,
/// `pic` (checked with `rustc --print target-spec-json --target <triple>`
/// for each triple in [`ARCHITECTURE.md`]'s Tier-1 table). `pycc_rt`
/// therefore keeps `crate-type = ["staticlib", "rlib"]` and gains no
/// `cdylib`: the artifact is one shared object holding the compiled module
/// *and* the runtime, not two linked against each other. The C shim is the
/// one piece that does need an explicit flag -- see [`ext_compile_args`].
///
/// [`ARCHITECTURE.md`]: https://github.com/rotnov/pycc/blob/main/docs/ARCHITECTURE.md
pub(crate) fn ext_link_args(platform: ExtLinkPlatform, libs: &Path) -> Vec<OsString> {
    match platform {
        // A macOS extension module is a Mach-O *bundle*, and its `Py*`
        // references are resolved by the already-loaded interpreter at
        // `dlopen` time. Without `-undefined dynamic_lookup` the static
        // linker rejects them outright; with a `-lpython` instead, the
        // artifact would load a *second* copy of libpython beside the
        // running one.
        ExtLinkPlatform::MacOs => vec![
            OsString::from("-bundle"),
            OsString::from("-undefined"),
            OsString::from("dynamic_lookup"),
        ],
        // ELF resolves undefined symbols lazily against the global scope
        // the interpreter already occupies, so `-shared` is correct here and
        // a `-lpython` is actively wrong for the same reason as above.
        //
        // `-Bsymbolic` binds every reference this artifact makes to a symbol
        // it defines itself, at link time. Without it, two pycc extension
        // modules loaded with `RTLD_GLOBAL` interpose on each other: ELF
        // resolves a defined global symbol through the *global* lookup
        // scope, so the second module's `pycc_ext_module_exec` and
        // `fnptr_<name>` references reach the first module's definitions and
        // it executes the wrong module's body. CPython imports without
        // `RTLD_GLOBAL` by default, but `sys.setdlopenflags` is public API
        // and some extensions set it, so the artifact cannot rely on the
        // loader's default. Mach-O and PE need no counterpart: a two-level
        // namespace records the defining library per reference, and a PE
        // exports only what `__declspec(dllexport)` names -- and both `ld64`
        // and `link.exe` reject the flag, so it stays on this arm alone.
        ExtLinkPlatform::Linux => vec![OsString::from("-shared"), OsString::from("-Wl,-Bsymbolic")],
        // Windows has no lazy global scope: every import must be bound at
        // link time through an import library. `python3.lib` is the
        // stable-ABI one, deliberately not the version-tagged
        // `python313.lib` -- the whole point of the artifact is that it
        // outlives one interpreter minor version.
        ExtLinkPlatform::Windows => {
            let mut args = vec![OsString::from("-shared"), OsString::from("-L")];
            args.push(libs.as_os_str().to_os_string());
            args.push(OsString::from("-lpython3"));
            args
        }
    }
}

mod carrier;
mod constructors;
pub(crate) use constructors::*;
mod exports;
pub(crate) use exports::*;
mod defaults;
pub(crate) use carrier::*;
mod export_name;
mod exports_inc;
#[cfg(test)]
pub(crate) use exports_inc::generate_exports_inc;
pub(crate) use exports_inc::generate_exports_inc_with_slots;
mod getset;
#[cfg(test)]
pub(crate) use getset::ExtGetset;
pub(crate) use getset::{ExtClassGetsets, collect_carrier_getsets};
mod inherited;
mod instance_copy;
mod keywords;
pub(crate) use export_name::*;
pub(crate) use instance_copy::*;
pub(crate) use keywords::{SourceSignatures, bind_ctor_keyword_names, bind_keyword_names};
mod method_types;
mod module_hooks;
pub(crate) use method_types::*;
pub(crate) use module_hooks::{EntryHooks, is_module_hook};
mod publication;
mod richcompare;
pub(crate) use richcompare::{ExtSlotDunders, SlotArtifact, collect_slot_dunders};
#[cfg(test)]
pub(crate) use richcompare::{SLOT_DUNDERS, is_slot_dunder_method};

use publication::namespace_owner;
pub(crate) use publication::{ExtPublishedClass, collect_class_publications};
mod wrappers;
pub(crate) use wrappers::*;

/// The exact C declaration of the generated tag-to-class lookup.
///
/// Shared on purpose (risk R3 of this change's plan): the *definition* is
/// emitted here while the forward declaration is hand-written in
/// [`SHIM_C`], and nothing links the two until a C compiler runs inside an
/// `#[ignore]`d test the coverage job never executes. Both the shim test
/// and the generated-text test assert this one constant, so a spelling or
/// parameter-type drift fails an ordinary `cargo test` instead.
pub(crate) const USER_EXCEPTION_LOOKUP_DECL: &str =
    "static PyObject *pycc_ext_user_exception_class(unsigned char tag)";

/// The exact C declaration of the generated eager-registration entry point,
/// called from `pycc_ext_exec_module` after the `.inc` is included (so it
/// needs no forward declaration, unlike [`USER_EXCEPTION_LOOKUP_DECL`]).
pub(crate) const USER_EXCEPTION_REGISTER_DECL: &str =
    "static int pycc_ext_register_exception_classes(PyObject *module)";

/// The two PEP 654 group classes. A class whose MRO reaches either one is
/// excluded from the table by [`collect_user_exception_classes`]: the
/// limited C API exposes no `PyExc_ExceptionGroup`, and PEP 654 requires
/// `(msg, exceptions)`, so a synthesized stand-in would be a fake group
/// class rather than CPython's. Such a class keeps today's honest
/// `Exception`; `docs/RUNTIME.md` records the residual.
const GROUP_EXCEPTION_CLASSES: [&str; 2] = ["BaseExceptionGroup", "ExceptionGroup"];

/// One base of a synthesized user exception class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExceptionBase {
    /// A seeded builtin exception class, spelled `PyExc_<name>` host-side.
    /// The `&'static str` is the [`BUILTIN_EXCEPTION_CLASSES`] entry itself,
    /// so the rendered spelling cannot drift from the array.
    Builtin(&'static str),
    /// Another entry of the same table, by its slot index. Always a lower
    /// slot than the class that names it: `resolve_mro` requires a base to
    /// be defined before its subclass, and `finalize` assigns tags in that
    /// same program order.
    User(usize),
}

/// One user-defined exception class the artifact synthesizes on the host
/// side, in the order the generated registration function creates them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UserExceptionClass {
    /// The runtime tag `pycc_ext_raise_pending` receives for an instance of
    /// this class. Assigned by `pycc_hir::program::finalize` in program
    /// order, so it shifts on any source edit: it is regenerated with every
    /// artifact and never persisted in a fixture, a checked-in file, or the
    /// hand-written shim.
    pub(crate) tag: u8,
    /// The class's own (unqualified) Python name.
    pub(crate) name: String,
    /// The exception bases, in declaration order. Provably non-empty: a
    /// class earns a tag only when its MRO reaches a builtin exception
    /// class, which requires at least one exception base.
    pub(crate) bases: Vec<ExceptionBase>,
}

/// The per-program table of user exception classes the artifact
/// synthesizes, in ascending tag order.
///
/// Membership is decided here and nowhere else (correction C3 of this
/// change's plan): the C shim performs no tag arithmetic of its own, so a
/// class this function omits simply misses the generated lookup and keeps
/// `PyExc_Exception`.
///
/// Selection is `exception_type_tag == Some(t)` with
/// `t >= FIRST_USER_EXCEPTION_TYPE_TAG`. Neither `None` nor a smaller
/// `Some` is a user class: `crates/pycc_hir/src/class.rs` documents that
/// trap directly -- the flat seven builtins carry `None` and every builtin
/// past them carries a fixed `Some` below `FIRST_USER_EXCEPTION_TYPE_TAG`.
/// Group-derived classes are
/// excluded by [`GROUP_EXCEPTION_CLASSES`], and a monomorphized generic
/// specialization is untagged by construction and never appears here.
///
/// Bases come from `def.bases`, never from the linearized `def.mro`:
/// `class E(MyBase, ValueError)` is legal and is caught natively by
/// `except ValueError:`, so taking the first exception ancestor out of the
/// MRO would pick `MyBase` alone and drop `ValueError` -- exactly the
/// host-side mismatch this table exists to remove. A non-exception base (a
/// method-only mixin) is dropped, which `docs/RUNTIME.md` records as a
/// residual.
pub(crate) fn collect_user_exception_classes(module: &HirModule) -> Vec<UserExceptionClass> {
    let selected: Vec<(&str, &[String], u8)> = module
        .class_defs
        .iter()
        .filter_map(|(name, def)| {
            let tag = def.exception_type_tag?;
            let derives_from_group = def
                .mro
                .iter()
                .any(|ancestor| GROUP_EXCEPTION_CLASSES.contains(&ancestor.as_str()));
            (tag >= FIRST_USER_EXCEPTION_TYPE_TAG && !derives_from_group).then_some((
                name.as_str(),
                def.bases.as_slice(),
                tag,
            ))
        })
        .collect();
    let slots: HashMap<&str, usize> = selected
        .iter()
        .enumerate()
        .map(|(slot, (name, _, _))| (*name, slot))
        .collect();
    selected
        .iter()
        .map(|(name, bases, tag)| UserExceptionClass {
            tag: *tag,
            name: (*name).to_string(),
            bases: bases
                .iter()
                .filter_map(|base| {
                    BUILTIN_EXCEPTION_CLASSES
                        .iter()
                        .copied()
                        .find(|builtin| *builtin == base.as_str())
                        .map(ExceptionBase::Builtin)
                        .or_else(|| slots.get(base.as_str()).copied().map(ExceptionBase::User))
                })
                .collect(),
        })
        .collect()
}

/// The C expression naming one base class at registration time.
fn base_expression(base: &ExceptionBase) -> String {
    match base {
        ExceptionBase::Builtin(name) => format!("PyExc_{name}"),
        ExceptionBase::User(slot) => format!("pycc_ext_user_exception_classes[{slot}]"),
    }
}

/// One table entry's registration: create the class if the cache slot is
/// empty, then publish it as a module attribute.
///
/// The cache owns the strong reference `PyErr_NewException` returns and
/// `PyModule_AddObjectRef` takes its own, so neither needs a compensating
/// decref. The slot is assigned *before* `AddObjectRef` is checked, so a
/// failing `AddObjectRef` cannot leak the class. A non-`NULL` slot is
/// reused rather than re-minted: deleting the `sys.modules` entry and
/// re-importing is the one path that re-runs `Py_mod_exec`
/// (`docs/RUNTIME.md`), and reuse keeps class identity stable across it.
/// A mid-registration failure leaves the already-created classes cached, so
/// a later import completes the remaining slots.
fn register_class_c(slot: usize, class: &UserExceptionClass) -> String {
    let name = &class.name;
    let mut out = format!("    if (pycc_ext_user_exception_classes[{slot}] == NULL) {{\n");
    // A single base is passed directly; two or more need a `PyTuple`, which
    // `PyErr_NewException` accepts and which is what gives the synthesized
    // class the same `__mro__` the native side matches on.
    let (base, release) = match class.bases.as_slice() {
        [single] => (base_expression(single), ""),
        several => {
            let packed: Vec<String> = several.iter().map(base_expression).collect();
            out.push_str(&format!(
                "        PyObject *bases = PyTuple_Pack({}, {});\n",
                several.len(),
                packed.join(", ")
            ));
            out.push_str("        if (bases == NULL) {\n            return -1;\n        }\n");
            ("bases".to_string(), "        Py_DECREF(bases);\n")
        }
    };
    out.push_str(&format!(
        "        pycc_ext_user_exception_classes[{slot}] = PyErr_NewException(\n            \
         PYCC_EXT_MODULE_NAME_STR \".{name}\", {base}, NULL);\n"
    ));
    out.push_str(release);
    out.push_str(&format!(
        "        if (pycc_ext_user_exception_classes[{slot}] == NULL) {{\n            \
         return -1;\n        }}\n    }}\n"
    ));
    out.push_str(&format!(
        "    if (PyModule_AddObjectRef(module, \"{name}\", \
         pycc_ext_user_exception_classes[{slot}]) < 0) {{\n        return -1;\n    }}\n"
    ));
    out
}

/// The user-exception-class half of the generated companion: the cache, the
/// tag lookup `pycc_ext_raise_pending` calls, and the eager registration
/// `pycc_ext_exec_module` calls.
///
/// Both functions are emitted unconditionally -- with empty bodies when the
/// program declares no user exception class -- so every artifact links.
///
/// Entries are keyed by **explicit tag** in a `switch`, not by an offset
/// into the cache: the tag space is sparse relative to the table, because
/// `finalize` consumes a tag for a group-derived class that
/// [`collect_user_exception_classes`] excludes. An array indexed by
/// `tag - FIRST_USER_EXCEPTION_TYPE_TAG` and sized by entry count would
/// then send a later class past its bounds and silently flatten it to
/// `Exception` with no compile or link error; a `switch` makes that
/// unrepresentable rather than merely tested against.
fn exception_classes_c(classes: &[UserExceptionClass]) -> String {
    let mut out = String::new();
    if classes.is_empty() {
        out.push_str(&format!(
            "{USER_EXCEPTION_LOOKUP_DECL}\n{{\n    (void)tag;\n    return NULL;\n}}\n\n"
        ));
        out.push_str(&format!(
            "{USER_EXCEPTION_REGISTER_DECL}\n{{\n    (void)module;\n    return 0;\n}}\n\n"
        ));
        return out;
    }
    out.push_str(&format!(
        "/* Synthesized user exception classes, created once during \
         `Py_mod_exec`.\n * File-scope statics are licensed by the shim's \
         refusal of both\n * subinterpreters and free-threaded hosts. */\n\
         static PyObject *pycc_ext_user_exception_classes[{}];\n\n",
        classes.len()
    ));
    out.push_str(&format!(
        "{USER_EXCEPTION_LOOKUP_DECL}\n{{\n    switch (tag) {{\n"
    ));
    for (slot, class) in classes.iter().enumerate() {
        out.push_str(&format!(
            "    case {}:\n        return pycc_ext_user_exception_classes[{slot}];\n",
            class.tag
        ));
    }
    out.push_str("    default:\n        return NULL;\n    }\n}\n\n");
    out.push_str(&format!("{USER_EXCEPTION_REGISTER_DECL}\n{{\n"));
    for (slot, class) in classes.iter().enumerate() {
        out.push_str(&register_class_c(slot, class));
    }
    out.push_str("    return 0;\n}\n\n");
    out
}

/// The `-I`, code-model and source arguments for the shim, in driver order.
/// Split out from [`ext_link_args`] so the platform arms there stay purely
/// about what makes the output loadable.
///
/// The ELF arm must pass `-fPIC` explicitly. GCC on Linux compiles to
/// position-dependent code by default, and the shim's references to
/// CPython's exception objects (`PyExc_ValueError`, `PyExc_ImportError`)
/// are undefined symbols resolved by the host at load time, so a
/// position-dependent `R_X86_64_PC32` relocation against them makes the
/// `-shared` link fail outright. It is emitted on the Mach-O arm too,
/// where clang already defaults to PIC, so the flag is a stated property
/// of the artifact rather than an inherited host default -- the same
/// reason this module takes its platform from the target triple and never
/// from `cfg!`. The Windows arm omits it: a PE/COFF target is position
/// independent by construction and its drivers warn the flag is ignored.
pub(crate) fn ext_compile_args(
    platform: ExtLinkPlatform,
    include: &Path,
    shim: &Path,
) -> Vec<OsString> {
    let mut args = vec![OsString::from("-I"), include.as_os_str().to_os_string()];
    if matches!(platform, ExtLinkPlatform::Linux | ExtLinkPlatform::MacOs) {
        args.push(OsString::from("-fPIC"));
    }
    args.push(shim.as_os_str().to_os_string());
    args
}

/// Borrowed view used only to keep [`ExtToolchain::interpreter`] observable
/// from a test without making the field public.
#[cfg(test)]
impl ExtToolchain {
    pub(crate) fn interpreter(&self) -> &OsStr {
        &self.interpreter
    }
}

#[cfg(test)]
#[path = "ext_build_tests/mod.rs"]
mod ext_build_tests;
