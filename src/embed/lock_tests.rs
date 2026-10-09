//! `plan_embed` consuming `pycc.lock` (#1242): the locked closure lands in
//! `OUT.pycc/closure/` with the launcher's `PYCC_EMBED_CLOSURE` define, and
//! every refusal leaves a previous sidecar untouched. Each lock is written
//! by `pycc lock` itself against a fake interpreter layout and fake site
//! directories, and the Linux arm runs no Mach-O tool, so no python3.14 and
//! no `otool` run.

use super::super::fake_layout::{FakeLayout, fake_layout, fake_windows_env};
use super::super::*;
use super::relative_files;
use crate::interop_policy::InteropCli;
use crate::lock::fixture::write_dist;
use crate::lock::probe::{LockProbe, parse_lock_probe, tests::probe_lines};
use pycc_scratch::ScratchDir;

const HOST: (&str, &str) = ("aarch64", "macos");

struct Env {
    root: PathBuf,
    _dir: ScratchDir,
    layout: FakeLayout,
    pure: PathBuf,
    /// Read only by the closure-image tests, which run on Unix hosts.
    #[cfg_attr(windows, expect(dead_code))]
    plat: PathBuf,
    lock_probe: LockProbe,
    entry: PathBuf,
}

impl Env {
    /// `tinypkg` requiring `tinydep`, both in purelib.
    fn new(tag: &str, body: &str) -> Self {
        let env = Self::bare(tag, body);
        write_dist(
            &env.pure,
            "tinypkg",
            "1.0",
            &[("tinypkg/__init__.py", b"from tinydep import X\n")],
            &["tinydep"],
        );
        write_dist(
            &env.pure,
            "tinydep",
            "2.0",
            &[("tinydep/__init__.py", b"X = 1\n")],
            &[],
        );
        env
    }

    /// Empty site directories.
    fn bare(tag: &str, body: &str) -> Self {
        Self::with_sites(tag, body, false)
    }

    /// Empty site directories inside the interpreter's prefix, as a
    /// non-venv interpreter has them (#1259).
    #[cfg(target_os = "macos")]
    fn in_prefix(tag: &str, body: &str) -> Self {
        Self::with_sites(tag, body, true)
    }

    fn with_sites(tag: &str, body: &str, in_prefix: bool) -> Self {
        let dir = ScratchDir::new(tag).unwrap();
        let root = std::fs::canonicalize(&*dir).unwrap();
        let layout_root = root.join("layout");
        std::fs::create_dir_all(&layout_root).unwrap();
        let layout = fake_layout(&layout_root);
        let sites = if in_prefix {
            layout.prefix.join("sites")
        } else {
            root.clone()
        };
        let pure = sites.join("pure");
        let plat = sites.join("plat");
        std::fs::create_dir_all(&pure).unwrap();
        std::fs::create_dir_all(&plat).unwrap();
        let lock_probe = parse_lock_probe(&probe_lines(&pure, &plat)).unwrap();
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let entry = project.join("m.py");
        std::fs::write(&entry, body).unwrap();
        Self {
            root,
            _dir: dir,
            layout,
            pure,
            plat,
            lock_probe,
            entry,
        }
    }

    fn toolchain(&self) -> EmbedToolchain {
        EmbedToolchain::with_probes("pyfake", self.layout.probe.clone(), self.lock_probe.clone())
            .with_windows_env(fake_windows_env(&self.layout))
    }

    fn lock(&self) {
        crate::lock::run_lock_on(
            &self.entry,
            false,
            InteropCli::default(),
            &self.toolchain(),
            HOST,
        )
        .unwrap_or_else(|_| panic!("`pycc lock` must succeed"));
    }

    #[cfg(unix)]
    fn lock_path(&self) -> PathBuf {
        self.entry.with_file_name("pycc.lock")
    }

    fn out(&self) -> PathBuf {
        self.root.join("app")
    }

    fn sidecar(&self) -> PathBuf {
        self.root.join("app.pycc")
    }

    fn obj(&self) -> PathBuf {
        self.root.join("main.o")
    }

    fn embed_with(&self, toolchain: &EmbedToolchain) -> Result<EmbedPlan, String> {
        self.embed_on(toolchain, EmbedPlatform::Linux)
    }

    fn embed_on(
        &self,
        toolchain: &EmbedToolchain,
        platform: EmbedPlatform,
    ) -> Result<EmbedPlan, String> {
        let hir = crate::frontend::lock_frontend(&self.entry, InteropCli::default())
            .unwrap_or_else(|_| panic!("the fixture must type-check"));
        plan_embed(
            &self.out(),
            &self.entry,
            &hir,
            toolchain,
            platform,
            HOST,
            &self.obj(),
        )
    }

    fn embed(&self) -> Result<EmbedPlan, String> {
        self.embed_with(&self.toolchain())
    }

    fn config(&self) -> String {
        std::fs::read_to_string(self.obj().with_file_name(EMBED_CONFIG_INC_NAME)).unwrap()
    }

    /// A marked sidecar from an earlier build, for the refusals to leave
    /// untouched.
    fn previous_sidecar(&self) {
        std::fs::create_dir(self.sidecar()).unwrap();
        std::fs::write(self.sidecar().join(layout::MARKER_NAME), "pycc-bundle 1\n").unwrap();
        std::fs::write(self.sidecar().join("previous.txt"), "kept").unwrap();
    }

    fn assert_previous_sidecar_intact(&self) {
        assert_eq!(
            relative_files(&self.sidecar()),
            [layout::MARKER_NAME, "previous.txt"]
        );
    }
}

#[test]
fn a_locked_closure_is_copied_into_the_sidecar_with_the_closure_define() {
    let env = Env::new("embed_lock_closure", "import tinypkg\n");
    env.lock();
    env.embed().expect("embedded");
    let files = relative_files(&env.sidecar());
    let closure: Vec<&str> = files
        .iter()
        .map(String::as_str)
        .filter(|rel| rel.starts_with("closure/"))
        .collect();
    assert!(
        closure.contains(&"closure/tinypkg/__init__.py"),
        "{files:?}"
    );
    assert!(
        closure.contains(&"closure/tinydep/__init__.py"),
        "{files:?}"
    );
    assert!(
        closure.contains(&"closure/tinypkg-1.0.dist-info/METADATA"),
        "{files:?}"
    );
    assert_eq!(
        std::fs::read(env.sidecar().join("closure/tinypkg/__init__.py")).unwrap(),
        b"from tinydep import X\n"
    );
    assert!(
        env.config().contains("#define PYCC_EMBED_CLOSURE"),
        "{}",
        env.config()
    );
}

#[test]
fn a_stdlib_only_program_gets_no_closure_directory_and_no_define() {
    // With a lock: its section is consumed (the library digest is checked)
    // but the closure is empty.
    let env = Env::new("embed_lock_stdlib", "import json\n");
    env.lock();
    env.embed().expect("embedded");
    assert!(!env.sidecar().join("closure").exists());
    assert!(
        !env.config().contains("PYCC_EMBED_CLOSURE"),
        "{}",
        env.config()
    );
    // Without one.
    std::fs::remove_file(env.entry.with_file_name("pycc.lock")).unwrap();
    env.embed().expect("embedded");
    assert!(!env.sidecar().join("closure").exists());
}

#[test]
fn a_missing_lock_is_refused_before_the_interpreter_is_probed() {
    let env = Env::new("embed_lock_missing", "import tinypkg\n");
    env.previous_sidecar();
    let toolchain = EmbedToolchain::with_interpreter("/nonexistent/pycc-test-python");
    let err = env.embed_with(&toolchain).unwrap_err();
    assert!(err.contains("which does not exist"), "{err}");
    assert!(err.contains("run `pycc lock"), "{err}");
    env.assert_previous_sidecar_intact();
}

#[test]
fn a_payload_file_edited_after_the_lock_is_refused_and_the_old_sidecar_kept() {
    let env = Env::new("embed_lock_edited", "import tinypkg\n");
    env.lock();
    env.previous_sidecar();
    std::fs::write(env.pure.join("tinydep/__init__.py"), "X = 2\n").unwrap();
    let err = env.embed().unwrap_err();
    assert!(err.contains("does not match its RECORD hash"), "{err}");
    assert!(err.contains("run `pycc lock"), "{err}");
    env.assert_previous_sidecar_intact();
}

#[test]
fn a_different_interpreter_library_is_refused_and_the_old_sidecar_kept() {
    for (tag, body) in [
        ("embed_lock_lib", "import tinypkg\n"),
        ("embed_lock_lib_stdlib", "import json\n"),
    ] {
        let env = Env::new(tag, body);
        env.lock();
        env.previous_sidecar();
        std::fs::write(env.layout.library(), "another library").unwrap();
        let err = env.embed().unwrap_err();
        assert!(err.contains("libpython-sha256"), "{err}");
        assert!(err.contains("run `pycc lock"), "{err}");
        env.assert_previous_sidecar_intact();
    }
}

#[test]
fn an_interpreter_field_mismatch_is_refused_after_the_probe() {
    let env = Env::new("embed_lock_interp", "import tinypkg\n");
    env.lock();
    let mut probe = env.layout.probe.clone();
    probe.version = (3, 14, 9);
    let toolchain = EmbedToolchain::with_probes("pyfake", probe, env.lock_probe.clone());
    let err = env.embed_with(&toolchain).unwrap_err();
    assert!(err.contains("python"), "{err}");
    assert!(err.contains("3.14.9"), "{err}");
}

#[cfg(unix)]
const LINUX: (&str, &str) = ("x86_64", "linux");

/// A Linux build over synthetic ELF images, on every Unix host (a Windows
/// path would split `DT_RUNPATH` at its drive colon): two distributions
/// whose extensions need `libnat1`, which needs `libnat2`, both outside the
/// interpreter and the system directory (#1243).
#[cfg(unix)]
fn linux_native_env(tag: &str) -> (Env, EmbedToolchain) {
    use super::super::elf::fixture::{ElfSpec, elf_bytes};
    let env = Env::bare(tag, "import tinynat\nimport tinyb\n");
    let write = |rel: &str, spec: &ElfSpec| {
        let path = env.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, elf_bytes(spec)).unwrap();
    };
    write("sys/libc.so.6", &ElfSpec::library("libc.so.6", &[]));
    let nat1 = ElfSpec::library("libnat1.so.1", &["libnat2.so.2"]).runpath("$ORIGIN");
    write("outside/libnat1.so.1", &nat1);
    write(
        "outside/libnat2.so.2",
        &ElfSpec::library("libnat2.so.2", &["libc.so.6"]),
    );
    let outside = env.root.join("outside").display().to_string();
    let ext = elf_bytes(&ElfSpec::module(&["libnat1.so.1", "libc.so.6"]).runpath(&outside));
    for dist in ["tinynat", "tinyb"] {
        let init = format!("{dist}/__init__.py");
        let so = format!("{dist}/_ext.so");
        let files: [(&str, &[u8]); 2] = [(&init, b"X = 1\n"), (&so, &ext)];
        write_dist(&env.plat, dist, "1.0", &files, &[]);
    }
    let linux = LinuxEnv {
        system_dirs: vec![env.root.join("sys")],
        ldconfig_programs: vec![env.root.join("absent")],
    };
    let toolchain = env.toolchain().with_linux_env(linux);
    (env, toolchain)
}

#[cfg(unix)]
fn lock_linux(env: &Env, toolchain: &EmbedToolchain, check: bool) -> Result<(), String> {
    use crate::lock::LockFailure::Stale;
    let result =
        crate::lock::run_lock_on(&env.entry, check, InteropCli::default(), toolchain, LINUX);
    // Any other failure is reported as an empty message, which no
    // assertion accepts.
    result.map_err(|f| if let Stale(m) = f { m } else { String::new() })
}

#[cfg(unix)]
fn embed_linux(env: &Env, toolchain: &EmbedToolchain) -> Result<EmbedPlan, String> {
    let hir = crate::frontend::lock_frontend(&env.entry, InteropCli::default())
        .unwrap_or_else(|_| panic!("the fixture must type-check"));
    let platform = EmbedPlatform::Linux;
    plan_embed(
        &env.out(),
        &env.entry,
        &hir,
        toolchain,
        platform,
        LINUX,
        &env.obj(),
    )
}

/// `pycc lock` records both natives for both distributions, the build
/// derives the same entries, copies both into `lib/` and links them into
/// the executable, and `pycc lock --check` agrees with what it wrote.
#[cfg(unix)]
#[test]
fn a_linux_build_copies_and_preloads_its_natives() {
    let (env, toolchain) = linux_native_env("embed_linux_native");
    lock_linux(&env, &toolchain, false).expect("locked");
    let lock =
        crate::lock::schema::parse(&std::fs::read_to_string(env.lock_path()).unwrap()).unwrap();
    let natives = &lock.target[0].native;
    let names: Vec<&str> = natives.iter().map(|native| native.name.as_str()).collect();
    assert_eq!(names, ["libnat1.so.1", "libnat2.so.2"]);
    assert!(
        natives
            .iter()
            .all(|native| native.required_by == ["tinyb", "tinynat"])
    );
    lock_linux(&env, &toolchain, true).expect("the lock is current");
    let plan = embed_linux(&env, &toolchain).expect("embedded");
    let lib = env.sidecar().join("lib");
    for name in names {
        let copied = std::fs::read(lib.join(name)).unwrap();
        assert_eq!(
            copied,
            std::fs::read(env.root.join("outside").join(name)).unwrap()
        );
    }
    let libs = ["libnat1.so.1".to_string(), "libnat2.so.2".to_string()];
    let preload = layout::preload_args(EmbedPlatform::Linux, &lib, &libs);
    assert!(plan.link_args.ends_with(&preload), "{:?}", plan.link_args);
}

/// A refusal only the build's own walk reaches stops a build whose lock is
/// current: an interpreter extension needing a library outside the prefix.
#[cfg(unix)]
#[test]
fn a_linux_interpreter_image_needing_an_outside_library_is_refused() {
    use super::super::elf::fixture::{ElfSpec, elf_bytes};
    let (env, toolchain) = linux_native_env("embed_linux_dynload");
    lock_linux(&env, &toolchain, false).expect("locked");
    let outside = env.root.join("outside").display().to_string();
    let dynload = env.layout.dynload();
    std::fs::create_dir_all(&dynload).unwrap();
    let module = ElfSpec::module(&["libnat2.so.2"]).runpath(&outside);
    std::fs::write(dynload.join("_x.so"), elf_bytes(&module)).unwrap();
    let err = embed_linux(&env, &toolchain).expect_err("refused");
    assert!(err.contains("is not relocatable"), "{err}");
    assert!(err.contains("lib-dynload/_x.so"), "{err}");
}

/// A native changed after `pycc lock` is refused by the build, which keeps
/// the previous sidecar, and reported stale by `pycc lock --check`.
#[cfg(unix)]
#[test]
fn a_linux_native_changed_after_the_lock_is_refused() {
    let (env, toolchain) = linux_native_env("embed_linux_native_stale");
    lock_linux(&env, &toolchain, false).expect("locked");
    let nat2 = env.root.join("outside/libnat2.so.2");
    let mut bytes = std::fs::read(&nat2).unwrap();
    bytes.push(0);
    std::fs::write(&nat2, bytes).unwrap();
    env.previous_sidecar();
    let err = embed_linux(&env, &toolchain).expect_err("stale");
    let differs = "`[[target.native]]` library `libnat2.so.2` differs";
    assert!(err.contains(differs), "{err}");
    env.assert_previous_sidecar_intact();
    let stale = lock_linux(&env, &toolchain, true).expect_err("stale");
    assert!(stale.contains(differs), "{stale}");
}

#[cfg(target_os = "macos")]
#[path = "macos_closure_tests.rs"]
mod macos_closure_tests;

#[cfg(target_os = "macos")]
#[path = "macos_relative_tests.rs"]
mod macos_relative_tests;

#[path = "static_lock_tests.rs"]
mod static_lock_tests;

/// A static libpython request on Windows is refused before the interpreter
/// is probed or the locked closure planned (D-253): the interpreter here
/// does not exist.
#[test]
fn a_windows_static_request_is_refused_before_the_probe() {
    let env = Env::new("embed_lock_windows_static", "import json\n");
    env.lock();
    env.previous_sidecar();
    let missing = EmbedToolchain::with_interpreter("/nonexistent/pycc-test-python")
        .with_link(LibpythonLink::Static);
    let err = env.embed_on(&missing, EmbedPlatform::Windows).unwrap_err();
    assert_eq!(err, windows::STATIC_REFUSAL);
    env.assert_previous_sidecar_intact();
}

#[path = "windows_lock_tests.rs"]
mod windows_lock_tests;
