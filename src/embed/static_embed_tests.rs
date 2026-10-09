//! `plan_embed` for a static libpython (D-251): the probe accepts a
//! static-only interpreter, the static probe finds and checks the archive,
//! the sidecar carries no libpython, the marker records the archive and
//! the link mode, and the link loads the whole archive. The Linux arm runs
//! on any host; the macOS relocation's refusals need real Mach-O images.

use super::super::fake_layout::{FakeLayout, fake_layout};
use super::super::*;
use super::{HOST, relative_files, typed};
use pycc_scratch::ScratchDir;

/// A static-only interpreter: no shared library is configured or present.
fn static_only(layout: &mut FakeLayout) {
    std::fs::remove_file(layout.library()).expect("remove the shared library");
    layout.probe.enable_shared = false;
    layout.probe.ldlibrary = "libpython3.14.a".to_string();
}

/// Writes a valid archive under `root` and returns its path.
fn archive(root: &Path) -> PathBuf {
    let archive = root.join("config").join("libpython3.14.a");
    std::fs::create_dir_all(archive.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&archive, b"!<arch>\nfake members").expect("write the archive");
    archive
}

fn toolchain(layout: &FakeLayout, archive: &Path) -> EmbedToolchain {
    EmbedToolchain::with_probe("pyfake", layout.probe.clone())
        .with_link(LibpythonLink::Static)
        .with_static_probe(StaticProbe {
            archive: archive.to_path_buf(),
            libs: vec!["-ldl".to_string(), "-lm".to_string()],
        })
}

fn embed(
    dir: &Path,
    toolchain: &EmbedToolchain,
    platform: EmbedPlatform,
) -> Result<EmbedPlan, String> {
    plan_embed(
        &dir.join("app"),
        &dir.join("m.py"),
        &typed(dir),
        toolchain,
        platform,
        HOST,
        &dir.join("main.o"),
    )
}

#[test]
fn a_static_linux_build_bundles_no_libpython_and_links_the_whole_archive() {
    let dir = ScratchDir::new("embed_static_linux").expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    let mut layout = fake_layout(&root);
    static_only(&mut layout);
    let archive = archive(&root);
    // The shared probe refuses this interpreter; the static one does not.
    let shared = EmbedToolchain::with_probe("pyfake", layout.probe.clone());
    assert!(
        shared
            .probe(EmbedPlatform::MacOs)
            .expect_err("shared")
            .contains("no shared libpython")
    );
    let plan = embed(&root, &toolchain(&layout, &archive), EmbedPlatform::Linux).expect("planned");
    let sidecar = root.join("app.pycc");
    assert_eq!(
        relative_files(&sidecar),
        [
            "PYCC-BUNDLE",
            "lib/python3.14/json/__init__.py",
            "lib/python3.14/os.py",
        ]
    );
    let marker = std::fs::read_to_string(sidecar.join("PYCC-BUNDLE")).expect("marker");
    assert!(layout::marker_is_current(&marker));
    let digest = sha256::sha256_hex(b"!<arch>\nfake members");
    assert!(
        marker.ends_with(&format!(
            "libpython-sha256 {digest}\nlibpython-link static\n"
        )),
        "{marker}"
    );
    let mut expected = layout::static_link_args(
        EmbedPlatform::Linux,
        &archive,
        &["-ldl".to_string(), "-lm".to_string()],
    );
    expected.extend(layout::rpath_args(EmbedPlatform::Linux, "app.pycc"));
    assert_eq!(plan.link_args, expected);
}

#[test]
fn a_static_build_refuses_a_missing_archive_and_a_vanished_one() {
    let dir = ScratchDir::new("embed_static_missing").expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    let mut layout = fake_layout(&root);
    static_only(&mut layout);
    let missing = root.join("config").join("libpython3.14.a");
    let err =
        embed(&root, &toolchain(&layout, &missing), EmbedPlatform::Linux).expect_err("no archive");
    assert!(err.contains("libpython3.14.a` does not exist"), "{err}");
    assert!(!root.join("app.pycc").exists());
    // The archive checked by the probe but gone before the digest.
    let err = bundle::assemble(
        &layout.probe,
        EmbedPlatform::Linux,
        &root,
        "app.pycc",
        false,
        None,
        &native::NativePlan::default(),
        Some(&StaticProbe {
            archive: missing.clone(),
            libs: Vec::new(),
        }),
    )
    .expect_err("unreadable archive");
    assert!(err.contains("could not read"), "{err}");
}

/// Writes an executable shell script standing in for an interpreter that
/// answers the static probe with `lines`, and anything else with garbage.
#[cfg(unix)]
fn static_interpreter(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("fake-static-python");
    std::fs::write(&script, format!("#!/bin/sh\n{body}")).expect("write the script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    script
}

#[cfg(unix)]
#[test]
fn a_spawned_static_probe_finds_checks_or_refuses_the_archive() {
    let dir = ScratchDir::new("embed_static_spawn").expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    let archive = archive(&root);
    let config = archive.parent().expect("a parent").display().to_string();
    let answer = format!(
        "case \"$3\" in\n*pycc-static-probe*) printf '%s\\n' '{config}' libpython3.14.a \
         '-ldl -lpthread' '-lm' ;;\n*) echo garbage ;;\nesac\n"
    );
    let static_toolchain =
        |script: PathBuf| EmbedToolchain::with_interpreter(script).with_link(LibpythonLink::Static);
    let probe = static_toolchain(static_interpreter(&root, &answer)).static_probe();
    assert_eq!(
        probe,
        Ok(StaticProbe {
            archive: archive.clone(),
            libs: vec!["-ldl".into(), "-lpthread".into(), "-lm".into()],
        })
    );
    let garbage = static_toolchain(static_interpreter(&root, "echo one-line\n")).static_probe();
    let garbage = garbage.expect_err("garbage");
    assert!(
        garbage.contains("did not report a configuration"),
        "{garbage}"
    );
    let failed = static_toolchain(static_interpreter(&root, "exit 5\n")).static_probe();
    assert!(failed.expect_err("failed").contains("exit 5"));
    let missing = static_toolchain(root.join("no-such-python")).static_probe();
    let missing = missing.expect_err("cannot start");
    assert!(
        missing.contains("could not run the embed interpreter"),
        "{missing}"
    );
}

#[cfg(target_os = "macos")]
mod macos {
    use super::super::super::fake_layout::{bundle as mach_bundle, macho_library};
    use super::*;

    /// On macOS the placeholder library is not an image, so the static
    /// relocation compares with no shared library, and an extension that
    /// needs none is bundled as it is.
    #[test]
    fn a_static_macos_build_bundles_no_libpython() {
        let dir = ScratchDir::new("embed_static_macos").expect("scratch");
        let root = std::fs::canonicalize(&*dir).expect("canonicalize");
        let layout = fake_layout(&root);
        mach_bundle(
            &layout.dynload().join("_plain.cpython-314-darwin.so"),
            "pycc_fake_plain",
            &[],
        );
        let archive = archive(&root);
        let plan =
            embed(&root, &toolchain(&layout, &archive), EmbedPlatform::MacOs).expect("planned");
        let files = relative_files(&root.join("app.pycc"));
        assert!(
            !files.iter().any(|rel| rel.contains("libpython")),
            "{files:?}"
        );
        assert!(
            files.contains(&"lib/python3.14/lib-dynload/_plain.cpython-314-darwin.so".to_string()),
            "{files:?}"
        );
        assert_eq!(plan.link_args[1], OsString::from("-force_load"));
    }

    /// An extension linking the shared library by its id is refused by
    /// name; one reaching it under another name is refused once the name
    /// resolves to it.
    #[test]
    fn a_static_macos_build_refuses_an_extension_that_needs_the_shared_library() {
        let dir = ScratchDir::new("embed_static_macos_refused").expect("scratch");
        let root = std::fs::canonicalize(&*dir).expect("canonicalize");
        let layout = fake_layout(&root);
        macho_library(&layout);
        let archive = archive(&root);
        let toolchain = toolchain(&layout, &archive);
        let err = embed(&root, &toolchain, EmbedPlatform::MacOs).expect_err("by name");
        assert!(err.contains("_json.cpython-314-darwin.so"), "{err}");
        assert!(
            err.contains("libpython3.14.dylib`, a shared libpython"),
            "{err}"
        );

        let alias = layout.prefix.join("lib").join("libalias.dylib");
        std::os::unix::fs::symlink(layout.library(), &alias).expect("symlink");
        let json = layout.dynload().join("_json.cpython-314-darwin.so");
        let library = layout.library().display().to_string();
        let change = [
            OsString::from("-change"),
            OsString::from(&library),
            alias.clone().into_os_string(),
            json.into_os_string(),
        ];
        bundle::run_tool("install_name_tool", &change).expect("change the dependency");
        let err = embed(&root, &toolchain, EmbedPlatform::MacOs).expect_err("by resolution");
        assert!(
            err.contains(&format!(
                "depends on `{}`, a shared libpython",
                alias.display()
            )),
            "{err}"
        );
    }
}
