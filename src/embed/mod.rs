//! The embedded-executable artifact mode (Part 1 of #1028): a plain
//! `pycc build` of a program with a standard-library CPython import links
//! a launcher that starts a bundled CPython 3.14 and runs the program as
//! its `__main__` module. The layout, the marker, the relocation rules and
//! the bridge split are recorded in the embedded-executable decision entry
//! under `docs/decisions/`; `docs/RUNTIME.md` describes the behavior.
//!
//! Kept out of `src/ext_build.rs` (already past the AGENTS.md size
//! threshold). It reuses that module's shim and `.inc` generator unchanged:
//! the shim stays the single layer that moves a `PyObject*` in both modes.

mod bundle;
mod closure;
mod elf;
#[cfg(test)]
pub(crate) mod fake_layout;
pub(crate) mod layout;
mod macho;
mod macho_host;
pub(crate) mod native;
pub(crate) mod native_linux;
mod native_windows;
mod native_windows_closure;
mod pe;
pub(crate) mod sha256;
pub(crate) mod static_lib;
pub(crate) mod stdlib_roots;
mod windows;

use crate::ext_build;
use crate::lock::probe::LockProbe;
use layout::EmbedPlatform;
pub(crate) use layout::LibpythonLink;
pub(crate) use native::plan_natives;
pub(crate) use native_linux::LinuxEnv;
pub(crate) use native_windows::WindowsEnv;
use static_lib::StaticProbe;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(windows)]
pub(crate) use windows::{StubLink, stub_link_args};

/// The launcher's C source, compiled into every embedded executable.
pub(crate) const LAUNCHER_C: &str = include_str!("pycc_embed_launcher.c");

/// The launcher's file name in the build's scratch directory.
pub(crate) const LAUNCHER_C_NAME: &str = "pycc_embed_launcher.c";

/// The generated header the launcher includes for its sidecar name.
pub(crate) const EMBED_CONFIG_INC_NAME: &str = "pycc_embed_config.inc";

/// The CPython minor line an embedded build bundles (D-128 pins 3.14).
pub(crate) const EMBED_PYTHON: (u32, u32) = (3, 14);

/// What the embed probe reports about an interpreter, one field per line
/// of [`EMBED_PROBE_SCRIPT`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmbedProbe {
    pub(crate) version: (u32, u32, u32),
    /// The interpreter's `sys.executable`, recorded in the marker.
    pub(crate) executable: PathBuf,
    pub(crate) include: PathBuf,
    pub(crate) stdlib: PathBuf,
    pub(crate) base_prefix: PathBuf,
    pub(crate) enable_shared: bool,
    pub(crate) framework: String,
    pub(crate) ldlibrary: String,
    pub(crate) libdir: PathBuf,
    pub(crate) instsoname: String,
    pub(crate) gil_disabled: bool,
}

impl EmbedProbe {
    /// The configuration values an error message prints, so a refusal
    /// says what the interpreter reported rather than only that it failed.
    fn describe(&self) -> String {
        let (major, minor, micro) = self.version;
        format!(
            "version {major}.{minor}.{micro}, Py_ENABLE_SHARED={}, PYTHONFRAMEWORK='{}', \
             LDLIBRARY='{}', LIBDIR='{}', INSTSONAME='{}', Py_GIL_DISABLED={}",
            u8::from(self.enable_shared),
            self.framework,
            self.ldlibrary,
            self.libdir.display(),
            self.instsoname,
            u8::from(self.gil_disabled)
        )
    }
}

/// Asks an interpreter what an embedded build needs, one value per line; a
/// config variable that is `None` prints as an empty line. Separate from
/// `ext_build`'s probe, whose contract and tests stay unchanged.
pub(crate) const EMBED_PROBE_SCRIPT: &str = "import sys,sysconfig\n\
     print('%d.%d.%d' % sys.version_info[:3])\n\
     print(sys.executable)\n\
     print(sysconfig.get_path('include'))\n\
     print(sysconfig.get_path('stdlib'))\n\
     print(sys.base_prefix)\n\
     for k in ('Py_ENABLE_SHARED','PYTHONFRAMEWORK','LDLIBRARY','LIBDIR','INSTSONAME','Py_GIL_DISABLED'):\n\
     \x20   v = sysconfig.get_config_var(k)\n\
     \x20   print('' if v is None else v)\n";

/// The interpreter an embedded build bundles, resolved lazily: nothing
/// spawns until [`Self::probe`] runs, which happens only once the program
/// is known to need an interpreter, so a native build never starts Python.
#[derive(Debug, Clone)]
pub(crate) struct EmbedToolchain {
    interpreter: OsString,
    /// How the executable links libpython (D-251); shared unless the build
    /// asked for a static libpython.
    link: LibpythonLink,
    probe_override: Option<EmbedProbe>,
    static_probe_override: Option<StaticProbe>,
    lock_probe_override: Option<LockProbe>,
    linux_env_override: Option<LinuxEnv>,
    windows_env_override: Option<WindowsEnv>,
}

impl EmbedToolchain {
    /// A toolchain for `interpreter` that runs every probe for real and
    /// links libpython shared.
    fn new(interpreter: OsString) -> Self {
        Self {
            interpreter,
            link: LibpythonLink::Shared,
            probe_override: None,
            static_probe_override: None,
            lock_probe_override: None,
            linux_env_override: None,
            windows_env_override: None,
        }
    }

    /// The production constructor: `PYCC_PYTHON` names the interpreter to
    /// bundle, default [`default_interpreter`] for the build host (not
    /// `--ext`'s `python3`, because an embedded build pins the 3.14 line).
    /// Reads the variable only.
    pub(crate) fn from_env() -> Self {
        let default = default_interpreter(EmbedPlatform::HOST);
        Self::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| OsString::from(default)))
    }

    /// The same toolchain, linking libpython as `link` says (D-251).
    pub(crate) fn with_link(mut self, link: LibpythonLink) -> Self {
        self.link = link;
        self
    }

    /// A toolchain that answers from `probe` instead of running anything.
    #[cfg(test)]
    pub(crate) fn with_probe(interpreter: impl Into<OsString>, probe: EmbedProbe) -> Self {
        let mut toolchain = Self::new(interpreter.into());
        toolchain.probe_override = Some(probe);
        toolchain
    }

    /// The same toolchain, answering the static probe from `probe` instead
    /// of running anything.
    #[cfg(test)]
    pub(crate) fn with_static_probe(mut self, probe: StaticProbe) -> Self {
        self.static_probe_override = Some(probe);
        self
    }

    /// A toolchain that answers both the embed probe and the lock probe
    /// from `probe` and `lock_probe` instead of running anything.
    #[cfg(test)]
    pub(crate) fn with_probes(
        interpreter: impl Into<OsString>,
        probe: EmbedProbe,
        lock_probe: LockProbe,
    ) -> Self {
        let mut toolchain = Self::with_probe(interpreter, probe);
        toolchain.lock_probe_override = Some(lock_probe);
        toolchain
    }

    /// The same toolchain, scanning Linux images against `env` instead of
    /// the build host's library directories.
    #[cfg(all(test, unix))]
    pub(crate) fn with_linux_env(mut self, env: LinuxEnv) -> Self {
        self.linux_env_override = Some(env);
        self
    }

    /// Where a Linux build's dependency scan looks for libraries.
    pub(crate) fn linux_env(&self) -> LinuxEnv {
        self.linux_env_override
            .clone()
            .unwrap_or_else(LinuxEnv::host)
    }

    /// The same toolchain, scanning Windows images against `env` instead
    /// of the build host's system directory. Not unix-only, unlike
    /// [`Self::with_linux_env`]: the Windows scan's tests run on every host.
    #[cfg(test)]
    pub(crate) fn with_windows_env(mut self, env: WindowsEnv) -> Self {
        self.windows_env_override = Some(env);
        self
    }

    /// Where a Windows build's interpreter scan looks for system DLLs.
    pub(crate) fn windows_env(&self) -> WindowsEnv {
        self.windows_env_override
            .clone()
            .unwrap_or_else(WindowsEnv::host)
    }

    /// A toolchain that really runs `interpreter`.
    #[cfg(test)]
    pub(crate) fn with_interpreter(interpreter: impl Into<OsString>) -> Self {
        Self::new(interpreter.into())
    }

    /// The interpreter's site directories, tags and marker environment
    /// (the lock probe), which `pycc lock` records and an embedded build
    /// compares against the lock.
    pub(crate) fn lock_probe(&self) -> Result<LockProbe, String> {
        match &self.lock_probe_override {
            Some(probe) => Ok(probe.clone()),
            None => crate::lock::probe::run_lock_probe(&self.interpreter),
        }
    }

    /// Probes the interpreter and checks it can be bundled for `platform`
    /// the way this toolchain links it, or returns an environment-failure
    /// message for exit 2.
    pub(crate) fn probe(&self, platform: EmbedPlatform) -> Result<EmbedProbe, String> {
        self.probe_as(self.link, platform)
    }

    /// [`Self::probe`] for an executable that links libpython as `link`
    /// says: a static link needs no shared library. `pycc lock` probes as
    /// static, because a lock serves either kind of build (#1272). On
    /// Windows the shared-library checks are replaced by
    /// [`windows::check_windows_layout`] (D-253): Windows CPython always
    /// ships a DLL, which it does not report as `Py_ENABLE_SHARED`.
    pub(crate) fn probe_as(
        &self,
        link: LibpythonLink,
        platform: EmbedPlatform,
    ) -> Result<EmbedProbe, String> {
        let probe = match &self.probe_override {
            Some(probe) => probe.clone(),
            None => self.run_probe()?,
        };
        check_embed_version(probe.version)?;
        let name = self.interpreter.to_string_lossy();
        if probe.gil_disabled {
            return Err(format!(
                "the embed interpreter `{name}` is a free-threaded build ({}); an embedded \
                 executable bundles a GIL-enabled CPython 3.14",
                probe.describe()
            ));
        }
        let windows = platform == EmbedPlatform::Windows;
        let shared = link == LibpythonLink::Shared && !windows;
        if shared && !check_shared(probe.enable_shared, &probe.framework) {
            return Err(format!(
                "the embed interpreter `{name}` has no shared libpython ({}); an embedded \
                 executable links against one -- set PYCC_PYTHON to a CPython 3.14 built \
                 with --enable-shared or as a framework",
                probe.describe()
            ));
        }
        if !probe.include.join("Python.h").is_file() {
            return Err(format!(
                "the embed interpreter `{name}` has no `Python.h` in `{}`; an embedded build \
                 compiles its launcher against that header -- install the interpreter's \
                 development files",
                probe.include.display()
            ));
        }
        if windows {
            windows::check_windows_layout(&probe, &name)?;
        }
        // A static build links the archive the static probe checks instead.
        // Windows never computes it: its `LIBDIR` is the import-library
        // directory, and `check_windows_layout` checked the DLL instead.
        if shared {
            let library = layout::source_library(&probe);
            if !library.is_file() {
                return Err(format!(
                    "the embed interpreter `{name}` reports a shared library `{}` that does \
                     not exist ({})",
                    library.display(),
                    probe.describe()
                ));
            }
        }
        if !probe.stdlib.is_dir() {
            return Err(format!(
                "the embed interpreter `{name}` reports a standard library `{}` that does not \
                 exist",
                probe.stdlib.display()
            ));
        }
        Ok(probe)
    }

    /// The interpreter's static libpython and the system libraries its
    /// members need, checked as [`static_lib::check_archive`] says, or an
    /// environment-failure message for exit 2. Runs only for a static
    /// build, after [`Self::probe`] accepted the interpreter.
    pub(crate) fn static_probe(&self) -> Result<StaticProbe, String> {
        self.static_probe_for(&static_lib::ArchiveUse::Link)
    }

    /// The file whose sha256 `pycc lock` records as `libpython-sha256` for
    /// the interpreter `probe` on `platform`
    /// ([`static_lib::identity_library`]). The static probe runs only for
    /// a POSIX interpreter without a shared libpython.
    pub(crate) fn identity_library(
        &self,
        probe: &EmbedProbe,
        platform: EmbedPlatform,
    ) -> Result<PathBuf, String> {
        static_lib::identity_library(probe, platform, || {
            let usage = static_lib::ArchiveUse::Identify {
                described: probe.describe(),
            };
            Ok(self.static_probe_for(&usage)?.archive)
        })
    }

    /// [`Self::static_probe`], with its refusals worded for `usage`.
    fn static_probe_for(&self, usage: &static_lib::ArchiveUse) -> Result<StaticProbe, String> {
        let probe = match &self.static_probe_override {
            Some(probe) => probe.clone(),
            None => self.run_script(
                static_lib::STATIC_PROBE_SCRIPT,
                static_lib::parse_static_probe,
            )?,
        };
        let name = self.interpreter.to_string_lossy();
        let archive = static_lib::check_archive(&name, &probe.archive, usage)?;
        Ok(StaticProbe {
            archive,
            libs: probe.libs,
        })
    }

    fn run_probe(&self) -> Result<EmbedProbe, String> {
        self.run_script(EMBED_PROBE_SCRIPT, parse_embed_probe)
    }

    /// Runs `script` under the interpreter and parses its output.
    fn run_script<T>(&self, script: &str, parse: fn(&str) -> Option<T>) -> Result<T, String> {
        let name = self.interpreter.to_string_lossy();
        let output = ext_build::probe_command(&self.interpreter, script)
            .output()
            .map_err(|e| {
                format!(
                    "could not run the embed interpreter `{name}`: {e}; a build that imports \
                     a CPython standard-library module bundles CPython 3.14 -- set \
                     PYCC_PYTHON to name one (default `{}`)",
                    default_interpreter(EmbedPlatform::HOST)
                )
            })?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed = output.status.success().then(|| parse(&stdout));
        parsed.flatten().ok_or_else(|| {
            format!(
                "the embed interpreter `{name}` did not report a configuration this build \
                 could parse (exit {})",
                output.status.code().unwrap_or(-1)
            )
        })
    }
}

/// Parses [`EMBED_PROBE_SCRIPT`]'s eleven lines. Pure.
pub(crate) fn parse_embed_probe(stdout: &str) -> Option<EmbedProbe> {
    let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
    let [
        version,
        executable,
        include,
        stdlib,
        base_prefix,
        shared,
        framework,
        ldlibrary,
        libdir,
        instsoname,
        gil,
    ]: [&str; 11] = lines.get(..11)?.try_into().ok()?;
    let mut parts = version.split('.').map(str::parse::<u32>);
    let version = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    Some(EmbedProbe {
        version,
        executable: PathBuf::from(executable),
        include: PathBuf::from(include),
        stdlib: PathBuf::from(stdlib),
        base_prefix: PathBuf::from(base_prefix),
        enable_shared: shared == "1",
        framework: framework.to_string(),
        ldlibrary: ldlibrary.to_string(),
        libdir: PathBuf::from(libdir),
        instsoname: instsoname.to_string(),
        gil_disabled: gil == "1",
    })
}

/// Accepts any `3.14.x`.
pub(crate) fn check_embed_version(version: (u32, u32, u32)) -> Result<(), String> {
    let (major, minor, micro) = version;
    if (major, minor) == EMBED_PYTHON {
        return Ok(());
    }
    Err(format!(
        "the embed interpreter is CPython {major}.{minor}.{micro}; an embedded executable \
         bundles CPython {}.{} -- set PYCC_PYTHON to a {}.{} interpreter",
        EMBED_PYTHON.0, EMBED_PYTHON.1, EMBED_PYTHON.0, EMBED_PYTHON.1
    ))
}

/// The interpreter an embedded build bundles when `PYCC_PYTHON` is unset:
/// `python3.14.exe` on Windows (D-253), `python3.14` elsewhere.
pub(crate) fn default_interpreter(platform: EmbedPlatform) -> &'static str {
    match platform {
        EmbedPlatform::Windows => "python3.14.exe",
        EmbedPlatform::MacOs | EmbedPlatform::Linux => "python3.14",
    }
}

/// Whether the interpreter has a shared libpython. Measured rule: a
/// framework build is shared even though it reports `Py_ENABLE_SHARED=0`.
pub(crate) fn check_shared(enable_shared: bool, framework: &str) -> bool {
    enable_shared || !framework.is_empty()
}

/// The comparison and hash slots of the program's carrier types (#1470),
/// or every `C0003` refusing one, worded for an embedded executable.
///
/// The embedded executable runs the `--ext` shim and companion, so its
/// `__main__` carriers get #1427's slots exactly as an `--ext` module's do,
/// and `pycc_types` can admit a carrier as a subscript key, comparison
/// operand and list-display element in both artifacts. `try_build` calls
/// this before [`plan_embed`], whose first probe needs an interpreter, so
/// the refusal needs none; [`plan_embed`] calls it again for the slots it
/// renders.
pub(crate) fn embed_slot_dunders(
    typed_hir: &pycc_hir::HirModule,
) -> Result<Vec<ext_build::ExtSlotDunders>, Vec<pycc_diag::Diagnostic>> {
    let carriers = ext_build::collect_carrier_classes(typed_hir);
    ext_build::collect_slot_dunders(typed_hir, &carriers, ext_build::SlotArtifact::Embedded)
}

/// What `try_build`'s single link site adds for an embedded executable.
#[derive(Debug)]
pub(crate) struct EmbedPlan {
    /// `-I <include>`, then [`layout::pic_args`] (`-fPIC`, none on
    /// Windows), then `<shim> <launcher>`, before the pycc object.
    pub(crate) compile_args: Vec<OsString>,
    /// The bundled library by path (or, for a static build, the archive
    /// and its system libraries), then the rpath, after the runtime. On
    /// Windows: `-shared`, the import library and `/NOIMPLIB` (D-253).
    pub(crate) link_args: Vec<OsString>,
    /// What the shared link writes instead of `OUT`: on Windows the program
    /// DLL `OUT.pycc\pycc_program.dll` (D-253); `None` elsewhere.
    pub(crate) artifact: Option<PathBuf>,
    /// On Windows, the stub `OUT` linked after the program DLL (D-253).
    #[cfg_attr(not(any(windows, test)), expect(dead_code))]
    pub(crate) stub: Option<windows::StubLink>,
}

/// Prepares everything an embedded build links, in order: the sidecar
/// name, the check of an existing sidecar (before anything is probed), the
/// `pycc.lock` checks that need no interpreter (#1242), the probe, the
/// lock's interpreter comparison and payload plan, the native libraries
/// (re-derived and compared with the section's, #1243), the C sources in the
/// scratch directory beside `obj_path`, and the sidecar itself (with the
/// locked closure copied into it), swapped into place before the link so
/// the executable links against the final bundled library. `host` is the
/// `(arch, os)` pair the lock section is selected by.
///
/// A static build (D-251) runs the static probe after the probe, bundles
/// no libpython, links the archive whole in the bundled library's place,
/// and checks a consumed lock section's `libpython-sha256` against the
/// file that identifies the interpreter (#1272), not the archive it links.
///
/// A Windows build (D-253) refuses a static libpython before the probe,
/// then refuses an interpreter image whose imports would not resolve once
/// moved (#1305) and a locked closure image or native whose imports no
/// strict rule places (#1306), all before anything is written; its
/// natives are copied into `OUT.pycc\natives\`; it compiles without `-fPIC`,
/// links the program DLL into the sidecar as [`EmbedPlan::artifact`], and
/// describes the stub `OUT` linked after it as [`EmbedPlan::stub`].
///
/// Precondition (#1470): [`embed_slot_dunders`] accepts `typed_hir`. The
/// build driver checks it first, so an uninstallable slot dunder is a
/// `C0003` before the probe; a caller that skips the check panics here.
pub(crate) fn plan_embed(
    out: &Path,
    entry: &Path,
    typed_hir: &pycc_hir::HirModule,
    toolchain: &EmbedToolchain,
    platform: EmbedPlatform,
    host: (&str, &str),
    obj_path: &Path,
) -> Result<EmbedPlan, String> {
    let sidecar_name = layout::sidecar_name(out)?;
    let parent = layout::sidecar_parent(out);
    let replace_existing = bundle::check_existing(&parent.join(&sidecar_name))?;
    let check = crate::lock::build::plan_closure(entry, typed_hir, host)?;
    windows::check_windows_request(platform, toolchain.link)?;
    let probe = toolchain.probe(platform)?;
    let static_lib = match toolchain.link {
        LibpythonLink::Static => Some(toolchain.static_probe()?),
        LibpythonLink::Shared => None,
    };
    let locked = match &check {
        Some(check) => {
            let lock_probe = toolchain.lock_probe()?;
            crate::lock::build::verify_interpreter(check, &probe, &lock_probe)?;
            Some(crate::lock::build::payload(check, &lock_probe, platform)?)
        }
        None => None,
    };
    let natives = plan_natives(
        platform,
        &probe,
        locked.as_ref(),
        &toolchain.linux_env(),
        &toolchain.windows_env(),
        true,
        toolchain.link,
    )?;
    if let Some(check) = &check {
        let difference = crate::lock::native_difference(&check.section.native, &natives.locked());
        if let Some(difference) = difference {
            return Err(check.stale(&difference));
        }
    }
    let has_closure = locked
        .as_ref()
        .is_some_and(|locked| !locked.files.is_empty());
    let shim = obj_path.with_file_name(ext_build::SHIM_C_NAME);
    let launcher = obj_path.with_file_name(LAUNCHER_C_NAME);
    let classes = ext_build::collect_user_exception_classes(typed_hir);
    let carriers = ext_build::collect_carrier_classes(typed_hir);
    // #1470: the slots only, never #1448's descriptors. A refused slot was
    // already reported by `try_build` through `embed_slot_dunders`.
    let slots = embed_slot_dunders(typed_hir).expect("try_build refuses an uninstallable slot");
    let exports_inc = ext_build::generate_exports_inc_with_slots(
        "__main__",
        &[],
        &classes,
        &[],
        &[],
        &carriers,
        &[],
        &slots,
    );
    write_source(&shim, ext_build::SHIM_C)?;
    let exports_path = obj_path.with_file_name(ext_build::EXPORTS_INC_NAME);
    write_source(&exports_path, &exports_inc)?;
    write_source(&launcher, LAUNCHER_C)?;
    let config_path = obj_path.with_file_name(EMBED_CONFIG_INC_NAME);
    let has_natives = platform == EmbedPlatform::Windows && !natives.natives.is_empty();
    let config = layout::embed_config_inc(&sidecar_name, has_closure, has_natives);
    write_source(&config_path, &config)?;
    let library = bundle::assemble(
        &probe,
        platform,
        parent,
        &sidecar_name,
        replace_existing,
        locked.as_ref(),
        &natives,
        static_lib.as_ref(),
    )?;
    let mut compile_args = vec![OsString::from("-I"), probe.include.clone().into_os_string()];
    compile_args.extend(layout::pic_args(platform));
    compile_args.extend([shim.into(), launcher.into()]);
    let is_windows = platform == EmbedPlatform::Windows;
    let mut link_args = match &static_lib {
        Some(archive) => layout::static_link_args(platform, &archive.archive, &archive.libs),
        None if is_windows => windows::program_link_args(&probe),
        None => vec![library.into_os_string()],
    };
    link_args.extend(layout::rpath_args(platform, &sidecar_name));
    let lib_dir = parent.join(&sidecar_name).join("lib");
    let names: Vec<String> = natives
        .linux_vendor
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    link_args.extend(layout::preload_args(platform, &lib_dir, &names));
    let sidecar = parent.join(&sidecar_name);
    let artifact = is_windows.then(|| sidecar.join(layout::PROGRAM_DLL_NAME));
    let stub = is_windows
        .then(|| windows::stub_link(obj_path, out))
        .transpose()?;
    Ok(EmbedPlan {
        compile_args,
        link_args,
        artifact,
        stub,
    })
}

fn write_source(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| {
        format!(
            "could not write the embedded build source `{}`: {e}",
            pycc_diag::display_path(&path.to_string_lossy())
        )
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
