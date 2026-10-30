//! The link of an `--ext` artifact: the per-platform arguments that make a
//! shared object an importable extension module ([`ext_link_args`]), and on
//! ELF the export set, section garbage collection and relocation packing
//! that keep it small and quick to load ([`ext_export_args`]).

use super::ExtLinkPlatform;
use std::ffi::OsString;
use std::path::Path;

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
/// the runtime's debug sections, which no loader reads.
///
/// `pack_relocs` ([`packs_relative_relocations`]) adds
/// `-z pack-relative-relocs` (v0.11): the module's relative relocations --
/// a pointer each in its method tables, type specs and chunk tables, about
/// 21,000 in pydantic's extension -- become a `DT_RELR` bitmap of a few
/// kilobytes instead of 24 bytes each, which took pydantic's extension from
/// 10.75 MB to 10.26 MB and its relocation entries from 21,222 to 349.
///
/// Empty for Mach-O and PE, whose exports are already explicit (and whose
/// fixups are already compact: chained fixups, base relocations).
pub(crate) fn ext_export_args(
    platform: ExtLinkPlatform,
    map: &Path,
    pack_relocs: bool,
) -> Vec<OsString> {
    match platform {
        ExtLinkPlatform::Linux => {
            let mut script = OsString::from("-Wl,--version-script=");
            script.push(map.as_os_str());
            let mut args = vec![
                script,
                OsString::from("-Wl,--gc-sections"),
                OsString::from("-Wl,--strip-debug"),
            ];
            if pack_relocs {
                args.push(OsString::from("-Wl,-z,pack-relative-relocs"));
            }
            args
        }
        ExtLinkPlatform::MacOs | ExtLinkPlatform::Windows => Vec::new(),
    }
}

/// Whether an ELF module built for `target` packs its relative relocations
/// ([`ext_export_args`]): only for the host (no `--target`), only when the
/// host's C library is glibc 2.36 or newer, the first to read `DT_RELR`,
/// and only when the host's linker really packs them
/// ([`host_linker_packs_relative_relocations`]): GNU ld 2.42, Ubuntu
/// 24.04's, implements `-z pack-relative-relocs` for x86 only and on
/// aarch64 ignores it with a warning. The linker then also records a
/// dependency on glibc's `GLIBC_ABI_DT_RELR` version, so the module refuses
/// to load, by name, on an older glibc rather than running unrelocated --
/// and the platform tag `pycc.build` gives a wheel (the host's glibc,
/// `python/pycc/_wheel.py`) already promises no older one. A cross build
/// cannot know its target's C library, and keeps the relocations unpacked.
pub(crate) fn packs_relative_relocations(target: Option<&str>) -> bool {
    target.is_none()
        && glibc_reads_dt_relr(host_glibc_version().as_deref())
        && host_linker_packs_relative_relocations()
}

/// Whether the host's `cc` packs relative relocations: [`linker_packs`],
/// asked once per process.
fn host_linker_packs_relative_relocations() -> bool {
    static PACKS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PACKS.get_or_init(|| linker_packs(std::process::Command::new("cc")))
}

/// Whether the C driver `cc` packs relative relocations: it links a shared
/// object with a few pointers in its data under `-z pack-relative-relocs`,
/// and the object must have a `DT_RELR` entry ([`has_dt_relr`]) -- a linker
/// that ignores the flag still links. Any failure (no scratch directory, a
/// failed compile or link) answers no, which keeps the plain link.
pub(crate) fn linker_packs(mut cc: std::process::Command) -> bool {
    let Ok(scratch) = pycc_scratch::ScratchDir::new("relr_probe") else {
        return false;
    };
    let source = scratch.join("probe.c");
    let object = scratch.join("probe.so");
    let probe = "int pycc_relr_probe[4];\n\
                 int *pycc_relr_pointers[4] = {&pycc_relr_probe[0], &pycc_relr_probe[1],\n\
                 &pycc_relr_probe[2], &pycc_relr_probe[3]};\n";
    if std::fs::write(&source, probe).is_err() {
        return false;
    }
    let linked = cc
        .args(["-shared", "-fPIC", "-Wl,-z,pack-relative-relocs"])
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    linked && std::fs::read(&object).is_ok_and(|elf| has_dt_relr(&elf))
}

/// Whether the ELF file `elf` (32- or 64-bit, either byte order) has a
/// `DT_RELR` entry in its dynamic section; `false` for anything that is not
/// a well-formed ELF file with one.
pub(crate) fn has_dt_relr(elf: &[u8]) -> bool {
    const PT_DYNAMIC: u64 = 2;
    const DT_NULL: u64 = 0;
    const DT_RELR: u64 = 36;
    if elf.get(..4) != Some(b"\x7fELF".as_slice()) {
        return false;
    }
    let wide = match elf.get(4) {
        Some(1) => false,
        Some(2) => true,
        _ => return false,
    };
    let big = match elf.get(5) {
        Some(1) => false,
        Some(2) => true,
        _ => return false,
    };
    let read = |at: usize, len: usize| -> Option<u64> {
        let bytes = elf.get(at..at.checked_add(len)?)?;
        let fold = |acc: u64, byte: &u8| (acc << 8) | u64::from(*byte);
        Some(if big {
            bytes.iter().fold(0, fold)
        } else {
            bytes.iter().rev().fold(0, fold)
        })
    };
    let word = if wide { 8 } else { 4 };
    let (phoff, phentsize, phnum) = if wide {
        (read(0x20, 8), read(0x36, 2), read(0x38, 2))
    } else {
        (read(0x1c, 4), read(0x2a, 2), read(0x2c, 2))
    };
    let (Some(phoff), Some(phentsize), Some(phnum)) = (phoff, phentsize, phnum) else {
        return false;
    };
    (0..phnum).any(|index| {
        let Some(header) = index
            .checked_mul(phentsize)
            .and_then(|at| at.checked_add(phoff))
            .and_then(|at| usize::try_from(at).ok())
        else {
            return false;
        };
        if read(header, 4) != Some(PT_DYNAMIC) {
            return false;
        }
        let (offset, size) = if wide {
            (read(header + 8, 8), read(header + 32, 8))
        } else {
            (read(header + 4, 4), read(header + 16, 4))
        };
        let (Some(Ok(offset)), Some(Ok(size))) =
            (offset.map(usize::try_from), size.map(usize::try_from))
        else {
            return false;
        };
        (0..size / (2 * word))
            .map(|entry| read(offset + entry * 2 * word, word))
            .take_while(|tag| !matches!(tag, None | Some(DT_NULL)))
            .any(|tag| tag == Some(DT_RELR))
    })
}

/// Whether the glibc whose version is `version` (`"2.39"`) reads `DT_RELR`
/// (2.36 and newer); `None` (not glibc) does not.
pub(crate) fn glibc_reads_dt_relr(version: Option<&str>) -> bool {
    let Some(version) = version else {
        return false;
    };
    let mut parts = version.split('.').map(|part| part.parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(major), Some(minor)) => (major, minor) >= (2, 36),
        _ => false,
    }
}

/// The running glibc's version (`gnu_get_libc_version`), or `None` on any
/// other C library.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub(crate) fn host_glibc_version() -> Option<String> {
    unsafe extern "C" {
        fn gnu_get_libc_version() -> *const std::ffi::c_char;
    }
    // SAFETY: glibc returns a pointer to a static, NUL-terminated string.
    let version = unsafe { std::ffi::CStr::from_ptr(gnu_get_libc_version()) };
    Some(version.to_string_lossy().into_owned())
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub(crate) fn host_glibc_version() -> Option<String> {
    None
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
