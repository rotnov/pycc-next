//! The macOS relocation against real Mach-O images: the `otool`,
//! `install_name_tool` and `codesign` spawns run on tiny libraries the test
//! builds with `cc`. macOS-only because the tools are; the coverage host is
//! macOS, so these lines still count.

use super::super::fake_layout::{bundle as mach_bundle, dylib, fake_layout, macho_library};
use super::*;

fn deps(image: &Path) -> Vec<String> {
    macho::parse_otool_l(
        &bundle::run_tool("otool", &[OsString::from("-L"), image.into()]).expect("otool"),
    )
}

/// Libpython's id becomes `@rpath`; an extension linking the source
/// libpython by its absolute id is rewritten to the bundled one, never
/// vendored; a prefix-internal library is vendored once for two users, and
/// its own prefix-internal dependency is vendored transitively.
#[test]
fn a_macos_bundle_relocates_bundles_and_vendors_every_image() {
    let dir = ScratchDir::new("embed_macos").expect("scratch");
    let layout = fake_layout(&dir);
    macho_library(&layout);
    let inner = layout.prefix.join("lib").join("libinner.dylib");
    dylib(&inner, "pycc_fake_inner", &[]);
    let vendor = layout.prefix.join("lib").join("libvendor.dylib");
    dylib(&vendor, "pycc_fake_vendor", &[&inner]);
    mach_bundle(
        &layout.dynload().join("_a.cpython-314-darwin.so"),
        "pycc_fake_a",
        &[&vendor],
    );
    mach_bundle(
        &layout.dynload().join("_b.cpython-314-darwin.so"),
        "pycc_fake_b",
        &[&vendor],
    );
    // An extension linking only system libraries needs no rewrite and so
    // no re-signing, and a dangling symlink in the standard library is
    // neither a directory nor a file and is not copied.
    let plain = layout.dynload().join("_plain.cpython-314-darwin.so");
    mach_bundle(&plain, "pycc_fake_plain", &[]);
    std::os::unix::fs::symlink("nowhere", layout.stdlib().join("dangling.py"))
        .expect("create a dangling symlink");
    let toolchain = EmbedToolchain::with_probe("pyfake", layout.probe.clone());
    let out = dir.join("app");
    let plan = plan_embed(
        &out,
        &dir.join("m.py"),
        &typed(&dir),
        &toolchain,
        EmbedPlatform::MacOs,
        HOST,
        &dir.join("main.o"),
    )
    .expect("planned");
    let lib = dir.join("app.pycc").join("lib");
    assert_eq!(
        plan.link_args[0],
        lib.join("libpython3.14.dylib").into_os_string()
    );
    assert_eq!(
        plan.link_args.last(),
        Some(&OsString::from("@executable_path/app.pycc/lib"))
    );
    assert_eq!(
        deps(&lib.join("libpython3.14.dylib"))[0],
        "@rpath/libpython3.14.dylib"
    );
    let dynload = lib.join("python3.14").join("lib-dynload");
    assert!(
        deps(&dynload.join("_json.cpython-314-darwin.so"))
            .contains(&"@rpath/libpython3.14.dylib".to_string())
    );
    for user in ["_a", "_b"] {
        let found = deps(&dynload.join(format!("{user}.cpython-314-darwin.so")));
        assert!(
            found.contains(&"@loader_path/../../libvendor.dylib".to_string()),
            "{found:?}"
        );
    }
    let vendored = deps(&lib.join("libvendor.dylib"));
    assert_eq!(vendored[0], "@rpath/libvendor.dylib");
    assert!(
        vendored.contains(&"@loader_path/libinner.dylib".to_string()),
        "{vendored:?}"
    );
    assert_eq!(
        deps(&lib.join("libinner.dylib"))[0],
        "@rpath/libinner.dylib"
    );
    assert!(
        !deps(&dynload.join("_plain.cpython-314-darwin.so"))
            .iter()
            .any(|dep| dep.starts_with('@')),
    );
    let dangling = lib.join("python3.14").join("dangling.py");
    assert!(std::fs::symlink_metadata(dangling).is_err());
    // Every rewritten image carries a valid ad-hoc signature again.
    for image in [
        lib.join("libpython3.14.dylib"),
        lib.join("libvendor.dylib"),
        dynload.join("_a.cpython-314-darwin.so"),
    ] {
        bundle::run_tool("codesign", &[OsString::from("-v"), image.into()])
            .expect("a valid signature");
    }
}

/// A dependency outside the interpreter and the system directories stops
/// the build, names the extension and the dependency, and leaves no
/// sidecar and no staging directory behind.
#[test]
fn a_dependency_outside_the_prefix_is_refused() {
    let dir = ScratchDir::new("embed_macos_refuse").expect("scratch");
    let layout = fake_layout(&dir);
    macho_library(&layout);
    let outside = std::fs::canonicalize(&*dir)
        .expect("canonical")
        .join("outside")
        .join("libout.dylib");
    dylib(&outside, "pycc_fake_out", &[]);
    mach_bundle(
        &layout.dynload().join("_ssl.cpython-314-darwin.so"),
        "pycc_fake_ssl",
        &[&outside],
    );
    let toolchain = EmbedToolchain::with_probe("pyfake", layout.probe.clone());
    let message = plan_embed(
        &dir.join("app"),
        &dir.join("m.py"),
        &typed(&dir),
        &toolchain,
        EmbedPlatform::MacOs,
        HOST,
        &dir.join("main.o"),
    )
    .expect_err("not relocatable");
    assert!(message.contains("is not relocatable"), "{message}");
    assert!(
        message.contains("_ssl.cpython-314-darwin.so") && message.contains("libout.dylib"),
        "{message}"
    );
    assert!(message.contains("pycc cannot bundle it"), "{message}");
    let leftovers: Vec<_> = std::fs::read_dir(&*dir)
        .expect("read_dir")
        .map(|entry| entry.expect("entry").file_name())
        .filter(|name| name.to_string_lossy().starts_with("app.pycc"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}
