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
/// time. See `src/ext/module/bridge.c`'s own header comment for why it is C
/// at all and why it is embedded rather than located by path. The shim is
/// kept as cohesive fragments under `src/ext/module/` (in this order, which
/// is the translation unit's order -- `instances.c` holds the
/// `#include "pycc_ext_exports.inc"`); `ext/pycc_ext_object_ops.c`
/// (non-strict mode's object operators, #3) is appended so the shim stays
/// one translation unit.
/// Only the tests read it whole; a build compiles the two units
/// [`shim_sources`] derives from these fragments (`shim_split`).
/// Placeholder parameter names `a0`, `a1`, ... for a hand-built export in a
/// test, one per parameter type.
#[cfg(test)]
pub(crate) fn test_param_names(params: &[Ty]) -> Vec<String> {
    (0..params.len()).map(|index| format!("a{index}")).collect()
}

#[cfg(test)]
pub(crate) const SHIM_C: &str = concat!(
    include_str!("ext/module/bridge.c"),
    include_str!("ext/module/frames.c"),
    include_str!("ext/module/boundary.c"),
    include_str!("ext/module/global_sync.c"),
    include_str!("ext/module/import_io.c"),
    include_str!("ext/module/object_unpack.c"),
    include_str!("ext/module/attr_cache.c"),
    include_str!("ext/module/object_ops.c"),
    include_str!("ext/module/object_index.c"),
    include_str!("ext/module/object_convert.c"),
    include_str!("ext/module/object_text.c"),
    include_str!("ext/module/object_stdlib.c"),
    include_str!("ext/module/object_intrinsics.c"),
    include_str!("ext/module/object_except.c"),
    include_str!("ext/module/object_closure.c"),
    include_str!("ext/module/ndarray.c"),
    include_str!("ext/module/bytearray.c"),
    include_str!("ext/module/carriers.c"),
    include_str!("ext/module/instances.c"),
    include_str!("ext/module/forward.c"),
    include_str!("ext/module/buffers.c"),
    include_str!("ext/module/module_ctx_head.c"),
    include_str!("ext/module/module_ctx.c"),
    include_str!("ext/module/module_init.c"),
    include_str!("ext/pycc_ext_object_ops.c")
);

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

/// The interpreter `--ext` probes when `PYCC_PYTHON` is unset: `python3`, or
/// `python.exe` on Windows, where the python.org installer and
/// `actions/setup-python` provide no `python3`.
pub(crate) const DEFAULT_PYTHON: &str = if cfg!(windows) {
    "python.exe"
} else {
    "python3"
};

impl ExtToolchain {
    /// The production constructor. `PYCC_PYTHON` names the interpreter to
    /// probe (default `python3`, `python.exe` on Windows, as for an embedded
    /// build); `PYCC_PYTHON_INCLUDE`, when set, supplies
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
            std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| OsString::from(DEFAULT_PYTHON));
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

    /// The interpreter that compiles the module's CPython source ahead of
    /// time (`precompiled`): the probed one, or none when the headers come
    /// from `PYCC_PYTHON_INCLUDE` (or a test's probe), where no interpreter
    /// need run on this host.
    pub(crate) fn precompile_interpreter(&self) -> Option<&OsStr> {
        self.probe_override
            .is_none()
            .then_some(self.interpreter.as_os_str())
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
        //
        // `/NOIMPLIB`, `/NOEXP`: nothing links against the module, so `link.exe`
        // writes no `m.lib` or `m.exp` beside it. The runtime's live-object
        // counters are exported as an ELF or Mach-O module exports every
        // global: a PE exports only what it names.
        ExtLinkPlatform::Windows => {
            let mut args = vec![OsString::from("-shared"), OsString::from("-L")];
            args.push(libs.as_os_str().to_os_string());
            args.push(OsString::from("-lpython3"));
            args.push(OsString::from("-Wl,/NOIMPLIB"));
            args.push(OsString::from("-Wl,/NOEXP"));
            args.extend(
                RUNTIME_COUNTERS
                    .iter()
                    .map(|name| OsString::from(format!("-Wl,/EXPORT:{name}"))),
            );
            args
        }
    }
}

/// The linker version script's file name in the build scratch directory
/// (ELF only, [`ext_export_args`]).
pub(crate) const EXPORT_MAP_NAME: &str = "pycc_ext_exports.map";

/// The ELF version script: the module exports its `PyInit_` function and
/// the runtime's live-object counters ([`RUNTIME_COUNTERS`]) and nothing
/// else, as a PE exports only what it names.
pub(crate) fn export_map() -> String {
    let mut map = String::from("{\n  global:\n    PyInit_*;\n");
    for name in RUNTIME_COUNTERS {
        map.push_str(&format!("    {name};\n"));
    }
    map.push_str("  local: *;\n};\n");
    map
}

/// The link arguments that keep an ELF module to the code it uses (v0.8):
/// with every runtime symbol local ([`export_map`], written to `map`),
/// `--gc-sections` drops the runtime functions nothing in the module
/// reaches, which took a pydantic module from 2.4 MB of code to 1.0 MB and
/// halved the time `dlopen` maps and relocates it; `--strip-debug` drops
/// the runtime's debug sections, which no loader reads. Empty for
/// Mach-O and PE, whose exports are already explicit.
pub(crate) fn ext_export_args(platform: ExtLinkPlatform, map: &Path) -> Vec<OsString> {
    match platform {
        ExtLinkPlatform::Linux => {
            let mut script = OsString::from("-Wl,--version-script=");
            script.push(map.as_os_str());
            vec![
                script,
                OsString::from("-Wl,--gc-sections"),
                OsString::from("-Wl,--strip-debug"),
            ]
        }
        ExtLinkPlatform::MacOs | ExtLinkPlatform::Windows => Vec::new(),
    }
}

/// The runtime's live-object counters, which a host (and the leak tests)
/// reads from an extension module with `ctypes`, and the attribute caches'
/// hit count (`src/ext/module/attr_cache.c`).
pub(crate) const RUNTIME_COUNTERS: [&str; 11] = [
    "pycc_rt_str_live_objects",
    "pycc_rt_list_live_objects",
    "pycc_rt_dict_live_objects",
    "pycc_rt_set_live_objects",
    "pycc_rt_instance_live_objects",
    "pycc_rt_exception_live_objects",
    "pycc_rt_closure_live_objects",
    "pycc_rt_gen_live_objects",
    "pycc_rt_num_box_live_objects",
    "pycc_rt_buffer_live_views",
    "pycc_ext_attr_cache_hits",
];

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
mod precompiled;
mod publication;
mod richcompare;
mod signatures;
pub(crate) use richcompare::{ExtSlotDunders, SlotArtifact, collect_slot_dunders};
#[cfg(test)]
pub(crate) use richcompare::{SLOT_DUNDERS, is_slot_dunder_method};
pub(crate) use signatures::{install as install_signatures, signature_chunk};
pub(crate) use precompiled::{install as install_precompiled, precompile};

use publication::namespace_owner;
pub(crate) use publication::{ExtPublishedClass, attach_class_bases, collect_class_publications};
mod wrappers;
pub(crate) use wrappers::*;
mod shim_split;

/// The runtime unit of the shim (`shim_split`), written beside
/// [`SHIM_C_NAME`]: the same text for every module, so its object is
/// compiled once and taken from the build cache afterwards.
pub(crate) const SHIM_RUNTIME_C_NAME: &str = "pycc_ext_runtime.c";

/// The C sources of an artifact whose generated companion is `inc_body`,
/// beside `obj_path`, with their contents: the module unit (at
/// [`SHIM_C_NAME`]), the companion it includes (at [`EXPORTS_INC_NAME`])
/// and the runtime unit (at [`SHIM_RUNTIME_C_NAME`]). The two `.c` files
/// are what is compiled, module unit first.
/// The shim's runtime unit (`shim_split`): one copy serves every module of
/// a package's extension, which the package build compiles itself.
pub(crate) fn runtime_unit_text() -> &'static str {
    shim_split::runtime_unit()
}

pub(crate) fn shim_sources(obj_path: &Path, inc_body: &str) -> [(PathBuf, String); 3] {
    [
        (
            obj_path.with_file_name(SHIM_C_NAME),
            shim_split::module_unit().to_string(),
        ),
        (
            obj_path.with_file_name(EXPORTS_INC_NAME),
            shim_split::relink(inc_body),
        ),
        (
            obj_path.with_file_name(SHIM_RUNTIME_C_NAME),
            shim_split::runtime_unit().to_string(),
        ),
    ]
}

/// The exact C declaration of the generated eager-registration entry point,
/// called from `pycc_ext_exec_module` after the `.inc` is included (so it
/// needs no forward declaration).
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
/// The C expression naming a compiled class's type object (`tp_name`):
/// `<module>.<Class>` for the entry module's own class, and the defining
/// module's dotted name for a class of another project module of the
/// program (v0.3 accept 1), as CPython names it.
pub(crate) fn type_spec_name(class: &str) -> String {
    match pycc_ast::namespace::split_module_global(class) {
        Some((module, plain)) => c_string_literal(&format!("{module}.{plain}")),
        None => format!("PYCC_EXT_MODULE_NAME_STR \".{class}\""),
    }
}

fn register_class_c(slot: usize, class: &UserExceptionClass) -> String {
    let name = &class.name;
    // A class another project module of the program defines (v0.3 accept
    // 1) is named after that module, as CPython names it, and is not an
    // attribute of this module.
    let qualified = pycc_ast::namespace::split_module_global(name);
    let type_name = type_spec_name(name);
    // `dataclasses.FrozenInstanceError` (`pycc_ast::desugar::dataclasses`)
    // is CPython's own class, so the host catches it by its real name.
    if name == "__pycc_FrozenInstanceError" {
        return format!(
            "    if (pycc_ext_user_exception_classes[{slot}] == NULL) {{\n        \
             PyObject *dataclasses = PyImport_ImportModule(\"dataclasses\");\n        \
             if (dataclasses == NULL) {{\n            return -1;\n        }}\n        \
             pycc_ext_user_exception_classes[{slot}] =\n            \
             PyObject_GetAttrString(dataclasses, \"FrozenInstanceError\");\n        \
             Py_DECREF(dataclasses);\n        \
             if (pycc_ext_user_exception_classes[{slot}] == NULL) {{\n            \
             return -1;\n        }}\n    }}\n"
        );
    }
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
         {type_name}, {base}, NULL);\n"
    ));
    out.push_str(release);
    out.push_str(&format!(
        "        if (pycc_ext_user_exception_classes[{slot}] == NULL) {{\n            \
         return -1;\n        }}\n    }}\n"
    ));
    if qualified.is_some() {
        return out;
    }
    out.push_str(&format!(
        "    if (PyModule_AddObjectRef(module, \"{name}\", \
         pycc_ext_user_exception_classes[{slot}]) < 0) {{\n        return -1;\n    }}\n"
    ));
    out
}

/// The user-exception-class half of the generated companion: the cache and
/// the eager registration `pycc_ext_exec_module` calls, which creates each
/// class, publishes it as a module attribute and enters it in the shim's
/// registry (`pycc_ext_user_exception_register`, `bridge.c`) under its tag,
/// where the tag lookups `pycc_ext_raise_pending` and the error bridge make
/// find it. One registry serves every module of a package's extension,
/// whose tags never overlap (`--exception-tag-base`).
///
/// The registration is emitted unconditionally -- with an empty body when
/// the program declares no user exception class -- so every artifact links.
/// The registry is keyed by **explicit tag**: the tag space is sparse
/// relative to the table, because `finalize` consumes a tag for a
/// group-derived class that [`collect_user_exception_classes`] excludes.
fn exception_classes_c(classes: &[UserExceptionClass]) -> String {
    let mut out = String::new();
    if classes.is_empty() {
        out.push_str(&format!(
            "{USER_EXCEPTION_REGISTER_DECL}\n{{\n    (void)module;\n    return 0;\n}}\n\n"
        ));
        out.push_str(&exception_class_named_c(classes));
        return out;
    }
    out.push_str(&format!(
        "/* Synthesized user exception classes, created once during \
         `Py_mod_exec`.\n * File-scope statics are licensed by the shim's \
         refusal of both\n * subinterpreters and free-threaded hosts. */\n\
         static PyObject *pycc_ext_user_exception_classes[{}];\n\n",
        classes.len()
    ));
    out.push_str(&format!("{USER_EXCEPTION_REGISTER_DECL}\n{{\n"));
    for (slot, class) in classes.iter().enumerate() {
        out.push_str(&register_class_c(slot, class));
    }
    for (slot, class) in classes.iter().enumerate() {
        out.push_str(&format!(
            "    pycc_ext_user_exception_register({}, pycc_ext_user_exception_classes[{slot}], \
             {});\n",
            class.tag,
            c_string_literal(&class.name)
        ));
    }
    out.push_str("    return 0;\n}\n\n");
    out.push_str(&exception_class_named_c(classes));
    out
}

/// The exact C declaration of the generated name-to-class map of the
/// exception classes compiled code tests with `isinstance` (the module
/// record's `exception_class_named`).
pub(crate) const EXCEPTION_CLASS_NAMED_DECL: &str =
    "static PyObject *pycc_ext_exception_class_named(const char *name)";

/// The CPython class standing for an exception class `isinstance(o, C)`
/// names in compiled code: one of the module's own synthesized classes
/// (`classes`, created by the registration above), else a builtin one by
/// its tag (`pycc_ext_exception_class_for_tag`), else NULL for a name that
/// is no exception class. A caught exception is a CPython object, so the
/// test is CPython's own `isinstance` against that class.
fn exception_class_named_c(classes: &[UserExceptionClass]) -> String {
    let mut out = format!("{EXCEPTION_CLASS_NAMED_DECL}\n{{\n");
    for (slot, class) in classes.iter().enumerate() {
        out.push_str(&format!(
            "    if (strcmp(name, {}) == 0) {{\n        \
             return pycc_ext_user_exception_classes[{slot}];\n    }}\n",
            c_string_literal(&class.name)
        ));
    }
    out.push_str("    static const char *const builtin[] = {");
    for name in pycc_hir::BUILTIN_EXCEPTION_CLASSES {
        out.push_str(&format!("{}, ", c_string_literal(name)));
    }
    out.push_str(&format!(
        "}};\n    for (int tag = 0; tag < {}; tag++) {{\n        \
         if (strcmp(name, builtin[tag]) == 0) {{\n            \
         return pycc_ext_exception_class_for_tag(tag);\n        }}\n    }}\n    \
         return NULL;\n}}\n\n",
        pycc_hir::BUILTIN_EXCEPTION_CLASSES.len()
    ));
    out
}

/// The optimization, `-I`, code-model and source arguments for the shim, in
/// driver order.
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
/// The C runtime flag every C object pycc links into a Windows image
/// compiles with (see [`ext_compile_args`]).
pub(crate) const WINDOWS_CRT_FLAG: &str = "-fms-runtime-lib=dll";

pub(crate) fn ext_compile_args(
    platform: ExtLinkPlatform,
    include: &Path,
    shim: &Path,
) -> Vec<OsString> {
    // Optimized: the shim's helpers run on every crossing between compiled
    // code and CPython objects, and unoptimized C keeps even CPython's
    // inline reference counting as calls.
    let mut args = vec![
        OsString::from("-O2"),
        OsString::from("-I"),
        include.as_os_str().to_os_string(),
    ];
    match platform {
        ExtLinkPlatform::Linux | ExtLinkPlatform::MacOs => args.push(OsString::from("-fPIC")),
        // A PE/COFF image is position-independent by construction. The DLL
        // C runtime: `pycc_rt.lib` is built with Rust's default, the DLL
        // runtime, and the link puts `ucrt.lib` first
        // (`add_windows_crt_math` in `src/build_pipeline.rs`), so every
        // object of the module agrees on one runtime.
        ExtLinkPlatform::Windows => args.push(OsString::from(WINDOWS_CRT_FLAG)),
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

/// `inc` with `PYCC_EXT_MODULE_DOC` defined as the entry module's docstring
/// when `source` opens with one, so the module object's `__doc__` is it, as
/// CPython sets it (`PyModuleDef.m_doc`); without one the shim keeps `NULL`
/// and `__doc__` is `None`, as for a module with no docstring.
pub(crate) fn with_module_doc(inc: String, source: &str) -> String {
    let Ok(module) = pycc_parser::parse_all(source) else {
        return inc;
    };
    let Some(pycc_ast::Stmt::Expr(first)) = module.body.first() else {
        return inc;
    };
    let pycc_ast::Expr::StringLiteral(doc) = first.value.as_ref() else {
        return inc;
    };
    let define = format!(
        "#define PYCC_EXT_MODULE_DOC {}\n",
        c_string_literal(doc.value.to_str())
    );
    match inc.find("\n\n") {
        Some(at) => format!("{}\n{define}{}", &inc[..at], &inc[at + 1..]),
        None => format!("{define}{inc}"),
    }
}

/// `text` as a C string literal: backslash, `"` and non-printable bytes escaped.
pub(crate) fn c_string_literal(text: &str) -> String {
    let mut out = String::from("\"");
    for byte in text.bytes() {
        match byte {
            b'"' | b'\\' => {
                out.push('\\');
                out.push(byte as char);
            }
            0x20..=0x7e => out.push(byte as char),
            _ => out.push_str(&format!("\\{byte:03o}")),
        }
    }
    out.push('"');
    out
}
