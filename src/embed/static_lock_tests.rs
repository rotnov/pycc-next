//! A static build consuming `pycc.lock` (Part 2 of #1227, #1272): the
//! lock's `libpython-sha256` names the file that identifies the
//! interpreter (its shared library, or its `LIBPL` archive when it has
//! none), a static build checks it against that file rather than the
//! archive it links, and a closure image that needs a shared libpython is
//! refused by name. Every lock is written by `pycc lock` itself against
//! the fake layout, as in `lock_tests.rs`.

use super::*;
use crate::lock::LockFailure;

/// Writes a valid archive under the scratch root and returns its path.
fn archive(env: &Env) -> PathBuf {
    let archive = env.root.join("config").join("libpython3.14.a");
    std::fs::create_dir_all(archive.parent().unwrap()).unwrap();
    std::fs::write(&archive, b"!<arch>\nfake members").unwrap();
    archive
}

/// The fake interpreter, linking statically from `archive`.
fn static_toolchain(env: &Env, probe: EmbedProbe, archive: &Path) -> EmbedToolchain {
    EmbedToolchain::with_probes("pyfake", probe, env.lock_probe.clone())
        .with_link(LibpythonLink::Static)
        .with_static_probe(StaticProbe {
            archive: archive.to_path_buf(),
            libs: Vec::new(),
        })
}

/// The layout's probe once its shared library is gone: an interpreter
/// built without one.
fn static_only(env: &Env) -> EmbedProbe {
    std::fs::remove_file(env.layout.library()).unwrap();
    let mut probe = env.layout.probe.clone();
    probe.enable_shared = false;
    probe.ldlibrary = "libpython3.14.a".to_string();
    probe
}

/// `pycc lock` under `toolchain`, with an environment failure's message.
fn lock_with(env: &Env, toolchain: &EmbedToolchain) -> Result<(), String> {
    let result =
        crate::lock::run_lock_on(&env.entry, false, InteropCli::default(), toolchain, HOST);
    // Any other failure is reported as an empty message, which no
    // assertion accepts.
    result.map_err(|f| match f {
        LockFailure::Env(message) => message,
        _ => String::new(),
    })
}

/// The `libpython-sha256` the lock recorded.
fn locked_digest(env: &Env) -> String {
    let text = std::fs::read_to_string(env.entry.with_file_name("pycc.lock")).unwrap();
    let lock = crate::lock::schema::parse(&text).unwrap();
    lock.target[0].libpython_sha256.clone()
}

fn marker(env: &Env) -> String {
    std::fs::read_to_string(env.sidecar().join(layout::MARKER_NAME)).unwrap()
}

/// A lock written for a shared interpreter serves a static build: the
/// closure is copied, no libpython is bundled, the marker records the
/// archive it links, and the lock keeps the shared library's digest.
#[test]
fn a_static_build_consumes_a_lock_and_copies_the_closure() {
    let env = Env::new("embed_static_lock", "import tinypkg\n");
    env.lock();
    let library = std::fs::read(env.layout.library()).unwrap();
    assert_eq!(locked_digest(&env), sha256::sha256_hex(&library));
    let archive = archive(&env);
    let toolchain = static_toolchain(&env, env.layout.probe.clone(), &archive);
    env.embed_with(&toolchain).expect("embedded");
    let files = relative_files(&env.sidecar());
    assert!(
        files.contains(&"closure/tinypkg/__init__.py".to_string()),
        "{files:?}"
    );
    assert!(
        !files.iter().any(|rel| rel.contains("libpython")),
        "{files:?}"
    );
    let digest = sha256::sha256_hex(b"!<arch>\nfake members");
    let marker = marker(&env);
    assert!(
        marker.ends_with(&format!(
            "libpython-sha256 {digest}\nlibpython-link static\n"
        )),
        "{marker}"
    );
    assert!(env.config().contains("#define PYCC_EMBED_CLOSURE"));
}

/// The static build compares the lock with the interpreter's shared
/// library, not the archive: the library changing after the lock is stale
/// for a closure and for a standard-library-only section alike, and the
/// previous sidecar is kept.
#[test]
fn a_static_build_refuses_a_lock_whose_interpreter_library_changed() {
    for (tag, body) in [
        ("embed_static_lock_lib", "import tinypkg\n"),
        ("embed_static_lock_lib_stdlib", "import json\n"),
    ] {
        let env = Env::new(tag, body);
        env.lock();
        env.previous_sidecar();
        std::fs::write(env.layout.library(), "another library").unwrap();
        let archive = archive(&env);
        let toolchain = static_toolchain(&env, env.layout.probe.clone(), &archive);
        let err = env.embed_with(&toolchain).unwrap_err();
        assert!(err.contains("libpython-sha256"), "{err}");
        assert!(err.contains("run `pycc lock"), "{err}");
        env.assert_previous_sidecar_intact();
    }
}

/// A shared-configured interpreter whose library vanished after the lock
/// is refused, not identified by its archive instead.
#[test]
fn a_static_build_refuses_a_configured_library_that_is_missing() {
    let env = Env::new("embed_static_lock_missing_lib", "import json\n");
    env.lock();
    env.previous_sidecar();
    std::fs::remove_file(env.layout.library()).unwrap();
    let archive = archive(&env);
    let toolchain = static_toolchain(&env, env.layout.probe.clone(), &archive);
    let err = env.embed_with(&toolchain).unwrap_err();
    assert!(
        err.ends_with("`pycc.lock` identifies the interpreter by that library's digest"),
        "{err}"
    );
    assert!(err.contains("reports a shared library"), "{err}");
    env.assert_previous_sidecar_intact();
}

/// `pycc lock` on an interpreter without a shared libpython records its
/// archive's digest; a static build then consumes the lock, and the
/// archive changing afterwards is stale.
#[test]
fn a_static_only_interpreter_is_locked_by_its_archive() {
    let env = Env::new("embed_static_only_lock", "import tinypkg\n");
    let probe = static_only(&env);
    let archive = archive(&env);
    let toolchain = static_toolchain(&env, probe.clone(), &archive);
    lock_with(&env, &toolchain).expect("locked");
    let digest = sha256::sha256_hex(b"!<arch>\nfake members");
    assert_eq!(locked_digest(&env), digest);
    env.embed_with(&toolchain).expect("embedded");
    assert!(env.sidecar().join("closure/tinypkg/__init__.py").is_file());
    // The marker's digest is the lock's: the archive both links and
    // identifies the interpreter.
    let marker = marker(&env);
    assert!(
        marker.ends_with(&format!(
            "libpython-sha256 {digest}\nlibpython-link static\n"
        )),
        "{marker}"
    );

    // A shared build of the same interpreter is still refused.
    let shared = EmbedToolchain::with_probes("pyfake", probe.clone(), env.lock_probe.clone());
    let err = env.embed_with(&shared).unwrap_err();
    assert!(err.contains("no shared libpython"), "{err}");

    std::fs::remove_dir_all(env.sidecar()).unwrap();
    env.previous_sidecar();
    std::fs::write(&archive, b"!<arch>\nother members").unwrap();
    let err = env.embed_with(&toolchain).unwrap_err();
    assert!(err.contains("libpython-sha256"), "{err}");
    assert!(err.contains("run `pycc lock"), "{err}");
    env.assert_previous_sidecar_intact();
}

/// `pycc lock` refuses an interpreter it cannot identify: no shared
/// libpython and no usable archive, or a configured shared library that
/// does not exist.
#[test]
fn pycc_lock_refuses_an_interpreter_it_cannot_identify() {
    let env = Env::new("embed_static_lock_refused", "import tinypkg\n");
    let shared_probe = env.layout.probe.clone();
    let probe = static_only(&env);
    let missing = env.root.join("config").join("libpython3.14.a");
    let err = lock_with(&env, &static_toolchain(&env, probe.clone(), &missing)).unwrap_err();
    assert!(
        err.starts_with(
            "the embed interpreter `pyfake` has no shared libpython (version 3.14.7, \
             Py_ENABLE_SHARED=0"
        ),
        "{err}"
    );
    assert!(
        err.ends_with("or one built with a shared libpython"),
        "{err}"
    );
    let text = env.root.join("text.a");
    std::fs::write(&text, "not an archive").unwrap();
    let err = lock_with(&env, &static_toolchain(&env, probe, &text)).unwrap_err();
    assert!(err.contains("is not an ar archive"), "{err}");

    let shared = EmbedToolchain::with_probes("pyfake", shared_probe, env.lock_probe.clone());
    let err = lock_with(&env, &shared).unwrap_err();
    assert!(err.contains("reports a shared library"), "{err}");
    assert!(
        err.ends_with("`pycc.lock` identifies the interpreter by that library's digest"),
        "{err}"
    );
    assert!(!env.entry.with_file_name("pycc.lock").exists());
}

/// A closure extension that needs a shared libpython is refused by name
/// in a static build, not as a stale lock: `libpython3.so` (the
/// stable-ABI shim) resolves to nothing here, and the lock itself is
/// current.
#[cfg(unix)]
#[test]
fn a_static_linux_build_refuses_a_closure_image_needing_libpython() {
    use super::super::super::elf::fixture::{ElfSpec, elf_bytes};
    let env = Env::bare("embed_static_lock_elf", "import tinyabi\n");
    let ext = elf_bytes(&ElfSpec::module(&["libpython3.so"]));
    let files: [(&str, &[u8]); 2] = [
        ("tinyabi/__init__.py", b"X = 1\n"),
        ("tinyabi/_ext.so", &ext),
    ];
    write_dist(&env.plat, "tinyabi", "1.0", &files, &[]);
    let linux = LinuxEnv {
        system_dirs: vec![env.root.join("sys")],
        ldconfig_programs: vec![env.root.join("absent")],
    };
    let probe = static_only(&env);
    let archive = archive(&env);
    let toolchain = static_toolchain(&env, probe, &archive).with_linux_env(linux);
    lock_linux(&env, &toolchain, false).expect("locked");
    env.previous_sidecar();
    let err = embed_linux(&env, &toolchain).expect_err("refused");
    assert!(err.contains("tinyabi/_ext.so"), "{err}");
    assert!(
        err.contains("depends on `libpython3.so`, a shared libpython"),
        "{err}"
    );
    assert!(!err.contains("pycc lock"), "{err}");
    env.assert_previous_sidecar_intact();
}

/// A static build vendors a locked closure's natives as a shared build
/// does: `pycc lock` under a static-only interpreter records both natives,
/// the build derives the same entries (so `--check` and the build agree),
/// copies them into `lib/` and preloads them, and bundles no libpython.
#[cfg(unix)]
#[test]
fn a_static_linux_build_vendors_its_locked_natives() {
    let (env, _) = linux_native_env("embed_static_linux_native");
    let linux = LinuxEnv {
        system_dirs: vec![env.root.join("sys")],
        ldconfig_programs: vec![env.root.join("absent")],
    };
    let probe = static_only(&env);
    let archive = archive(&env);
    let toolchain = static_toolchain(&env, probe, &archive).with_linux_env(linux);
    lock_linux(&env, &toolchain, false).expect("locked");
    lock_linux(&env, &toolchain, true).expect("the lock is current");
    let lock =
        crate::lock::schema::parse(&std::fs::read_to_string(env.lock_path()).unwrap()).unwrap();
    let names: Vec<String> = lock.target[0]
        .native
        .iter()
        .map(|native| native.name.clone())
        .collect();
    assert_eq!(names, ["libnat1.so.1", "libnat2.so.2"]);
    let plan = embed_linux(&env, &toolchain).expect("embedded");
    let lib = env.sidecar().join("lib");
    for name in &names {
        let copied = std::fs::read(lib.join(name)).unwrap();
        assert_eq!(
            copied,
            std::fs::read(env.root.join("outside").join(name)).unwrap()
        );
    }
    let preload = layout::preload_args(EmbedPlatform::Linux, &lib, &names);
    assert!(plan.link_args.ends_with(&preload), "{:?}", plan.link_args);
    let files = relative_files(&env.sidecar());
    assert!(
        !files.iter().any(|rel| rel.contains("libpython")),
        "{files:?}"
    );
    assert!(marker(&env).ends_with("libpython-link static\n"));
}

/// Real Mach-O images: the derivation and the relocation meet a static
/// interpreter that has no shared library for `otool` to read.
#[cfg(target_os = "macos")]
mod macos {
    use super::super::macos_closure_tests::image;
    use super::*;

    /// A static-only interpreter and a closure with a Mach-O extension:
    /// `pycc lock` and the build derive the natives without asking `otool`
    /// about the missing shared library.
    #[test]
    fn a_static_only_macos_closure_is_locked_and_built() {
        let env = Env::bare("embed_static_macos_lock", "import tinynat\n");
        let build = env.root.join("build");
        let ext = image(&build, "_ext.so", "-bundle", None, &[], &[]);
        let files: [(&str, &[u8]); 2] = [
            ("tinynat/__init__.py", b"X = 1\n"),
            ("tinynat/_ext.so", &ext),
        ];
        write_dist(&env.plat, "tinynat", "1.0", &files, &[]);
        let probe = static_only(&env);
        let archive = archive(&env);
        let toolchain = static_toolchain(&env, probe, &archive);
        lock_with(&env, &toolchain).expect("locked");
        env.embed_on(&toolchain, EmbedPlatform::MacOs)
            .expect("embedded");
        assert!(env.sidecar().join("closure/tinynat/_ext.so").is_file());
    }

    /// A closure extension linked against a framework `Python` outside
    /// the interpreter is refused by name as a second libpython, not
    /// reported as a stale lock.
    #[test]
    fn a_static_macos_build_refuses_a_foreign_framework_python() {
        let env = Env::bare("embed_static_macos_framework", "import tinynat\n");
        let build = env.root.join("build");
        let framework = env
            .root
            .join("elsewhere/Python.framework/Versions/3.14/Python");
        let id = framework.display().to_string();
        std::fs::create_dir_all(framework.parent().unwrap()).unwrap();
        let python = image(&build, "Python", "-dynamiclib", Some(&id), &[], &[]);
        std::fs::write(&framework, python).unwrap();
        let ext = image(&build, "_ext.so", "-bundle", None, &[&framework], &[]);
        let files: [(&str, &[u8]); 2] = [
            ("tinynat/__init__.py", b"X = 1\n"),
            ("tinynat/_ext.so", &ext),
        ];
        write_dist(&env.plat, "tinynat", "1.0", &files, &[]);
        let archive = archive(&env);
        let toolchain = static_toolchain(&env, env.layout.probe.clone(), &archive);
        lock_with(&env, &toolchain).expect("locked");
        env.previous_sidecar();
        let err = env
            .embed_on(&toolchain, EmbedPlatform::MacOs)
            .expect_err("refused");
        assert!(
            err.contains(&format!("depends on `{id}`, a shared libpython")),
            "{err}"
        );
        assert!(!err.contains("run `pycc lock"), "{err}");
        env.assert_previous_sidecar_intact();
    }
}
