//! The macOS relocation of locked-closure images (the pycc.lock decision
//! entry, rule 8; #1242) against real Mach-O images the test builds with
//! `cc`: each dependency arm on one build, a payload match rebound past
//! an absolute rpath, the natives, and a native's refusal. The
//! distribution is installed and locked by the test itself, so no
//! python3.14 runs. `macos_relative_tests.rs` covers the rest of #1259.

use super::*;
use crate::embed::fake_layout::{cc, dylib, macho_library};

pub(super) fn deps(image: &Path) -> Vec<String> {
    macho::parse_otool_l(
        &bundle::run_tool("otool", &[OsString::from("-L"), image.into()]).expect("otool"),
    )
}

/// Builds `<build>/<name>` (a `-dynamiclib` or `-bundle`), optionally with
/// an install name, linked against `links` and carrying `rpaths`, and
/// returns its bytes for the distribution's payload.
pub(super) fn image(
    build: &Path,
    name: &str,
    kind: &str,
    id: Option<&str>,
    links: &[&Path],
    rpaths: &[&str],
) -> Vec<u8> {
    std::fs::create_dir_all(build).unwrap();
    let source = build.join(format!("{name}.c"));
    std::fs::write(
        &source,
        format!(
            "int pycc_fake_{}(void) {{ return 3; }}\n",
            name.replace('.', "_")
        ),
    )
    .unwrap();
    let out = build.join(name);
    let mut args: Vec<String> = vec![kind.into(), "-o".into(), out.display().to_string()];
    if let Some(id) = id {
        args.extend(["-install_name".to_string(), id.to_string()]);
    }
    args.push(source.display().to_string());
    args.extend(links.iter().map(|link| link.display().to_string()));
    args.extend(rpaths.iter().map(|rpath| format!("-Wl,-rpath,{rpath}")));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    cc(build, &args);
    std::fs::read(out).unwrap()
}

fn macos_env(tag: &str) -> (Env, PathBuf) {
    let env = Env::bare(tag, "import tinynat\n");
    macho_library(&env.layout);
    let build = env.root.join("build");
    (env, build)
}

#[test]
fn every_closure_dependency_arm_relocates_on_one_build() {
    let (env, build) = macos_env("embed_macos_closure");
    let vendor = env.layout.prefix.join("lib").join("libvendor.dylib");
    dylib(&vendor, "pycc_fake_vendor", &[]);
    // Arm 0: each dylib keeps its own id.
    let helper = image(
        &build,
        "libhelper.dylib",
        "-dynamiclib",
        Some("@rpath/libhelper.dylib"),
        &[],
        &[],
    );
    let sib = image(
        &build,
        "libsib.dylib",
        "-dynamiclib",
        Some("@loader_path/.dylibs/libsib.dylib"),
        &[],
        &[],
    );
    // Arms 1, 2, 4 and 5: libSystem kept, the source libpython rewritten,
    // `@rpath` kept through its `@loader_path` rpath, the prefix library
    // vendored.
    let ext = image(
        &build,
        "ext_so",
        "-bundle",
        None,
        &[
            &build.join("libhelper.dylib"),
            &env.layout.library(),
            &vendor,
        ],
        &["@loader_path/.dylibs"],
    );
    // Arm 3: an `@loader_path` sibling in the image's own payload.
    let sib_user = image(
        &build,
        "sib_so",
        "-bundle",
        None,
        &[&build.join("libsib.dylib")],
        &[],
    );
    let class = [0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 0x34];
    write_dist(
        &env.plat,
        "tinynat",
        "1.0",
        &[
            ("tinynat/__init__.py", b"X = 1\n"),
            ("tinynat/.dylibs/libhelper.dylib", &helper),
            ("tinynat/.dylibs/libsib.dylib", &sib),
            ("tinynat/_ext.so", &ext),
            ("tinynat/_sib.so", &sib_user),
            ("tinynat/Main.class", &class),
            ("tinynat/tool", b"#!/bin/sh\n"),
        ],
        &[],
    );
    {
        use std::os::unix::fs::PermissionsExt;
        let tool = env.plat.join("tinynat/tool");
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    env.lock();
    env.embed_on(&env.toolchain(), EmbedPlatform::MacOs)
        .expect("embedded");

    let sidecar = env.sidecar();
    let closure = sidecar.join("closure").join("tinynat");
    let ext_deps = deps(&closure.join("_ext.so"));
    for expected in [
        "@rpath/libhelper.dylib",
        "@rpath/libpython3.14.dylib",
        "@loader_path/../../lib/libvendor.dylib",
        "/usr/lib/libSystem.B.dylib",
    ] {
        assert!(ext_deps.contains(&expected.to_string()), "{ext_deps:?}");
    }
    assert_eq!(
        deps(&sidecar.join("lib").join("libvendor.dylib"))[0],
        "@rpath/libvendor.dylib"
    );
    assert!(
        deps(&closure.join("_sib.so")).contains(&"@loader_path/.dylibs/libsib.dylib".to_string())
    );
    assert_eq!(
        deps(&closure.join(".dylibs/libhelper.dylib"))[0],
        "@rpath/libhelper.dylib"
    );
    assert_eq!(
        deps(&closure.join(".dylibs/libsib.dylib"))[0],
        "@loader_path/.dylibs/libsib.dylib"
    );
    bundle::run_tool(
        "codesign",
        &[OsString::from("-v"), closure.join("_ext.so").into()],
    )
    .expect("the rewritten image is signed again");
    // Neither a `.class` file nor a script is a Mach-O image; both are
    // copied byte for byte, the script still executable.
    assert_eq!(std::fs::read(closure.join("Main.class")).unwrap(), class);
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(closure.join("tool"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }
}

/// An `@rpath` whose first rpath is absolute (it may resolve outside the
/// bundle on the machine that runs the program) and absent on the host,
/// matched by another locked distribution's payload file: `tinyhelp`'s
/// RECORD lists `tinynat/.dylibs/libhelper.dylib`, so it co-owns root
/// `tinynat` and is locked. The reference is rebound to an explicit path
/// to the closure copy, and nothing is vendored (#1259).
#[test]
fn a_payload_match_behind_an_absolute_rpath_is_rebound() {
    let (env, build) = macos_env("embed_macos_closure_rpath");
    let helper = image(
        &build,
        "libhelper.dylib",
        "-dynamiclib",
        Some("@rpath/libhelper.dylib"),
        &[],
        &[],
    );
    let ext = image(
        &build,
        "ext_so",
        "-bundle",
        None,
        &[&build.join("libhelper.dylib")],
        &["/pycc-test-nowhere/lib", "@loader_path/.dylibs"],
    );
    write_dist(
        &env.plat,
        "tinyhelp",
        "1.0",
        &[("tinynat/.dylibs/libhelper.dylib", &helper)],
        &[],
    );
    write_dist(
        &env.plat,
        "tinynat",
        "1.0",
        &[
            ("tinynat/__init__.py", b"X = 1\n"),
            ("tinynat/_ext.so", &ext),
        ],
        &[],
    );
    env.lock();
    let lock =
        crate::lock::schema::parse(&std::fs::read_to_string(env.lock_path()).unwrap()).unwrap();
    let packages: Vec<&str> = lock.target[0]
        .package
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(packages, ["tinyhelp", "tinynat"]);
    assert!(lock.target[0].native.is_empty());
    env.embed_on(&env.toolchain(), EmbedPlatform::MacOs)
        .expect("embedded");
    let closure = env.sidecar().join("closure").join("tinynat");
    let ext_deps = deps(&closure.join("_ext.so"));
    assert!(
        ext_deps.contains(&"@loader_path/.dylibs/libhelper.dylib".to_string()),
        "{ext_deps:?}"
    );
    assert!(!ext_deps.contains(&"@rpath/libhelper.dylib".to_string()));
    assert!(!env.sidecar().join("lib").join("libhelper.dylib").exists());
}

/// Two distributions whose extensions link `libout1`, which links
/// `libout2`, both outside the interpreter and the system directories.
/// Returns the environment and the two natives' paths.
fn native_env(tag: &str) -> (Env, PathBuf, PathBuf) {
    let env = Env::bare(tag, "import tinynat\nimport tinyb\n");
    macho_library(&env.layout);
    let build = env.root.join("build");
    let out2 = env.root.join("outside").join("libout2.dylib");
    let out1 = env.root.join("outside").join("libout1.dylib");
    dylib(&out2, "pycc_fake_out2", &[]);
    dylib(&out1, "pycc_fake_out1", &[&out2]);
    for dist in ["tinynat", "tinyb"] {
        let ext = image(
            &build,
            &format!("{dist}_so"),
            "-bundle",
            None,
            &[&out1],
            &[],
        );
        let init = format!("{dist}/__init__.py");
        let so = format!("{dist}/_ext.so");
        let files: [(&str, &[u8]); 2] = [(&init, b"X = 1\n"), (&so, &ext)];
        write_dist(&env.plat, dist, "1.0", &files, &[]);
    }
    (env, out1, out2)
}

/// A library outside the interpreter that closure images link by absolute
/// install name, and the library it links in turn, are locked as natives
/// required by both distributions, vendored into `lib/` with `@rpath` ids,
/// and every reference to them is rewritten to the vendored copy (#1243).
#[test]
fn outside_libraries_are_locked_as_natives_and_vendored() {
    let (env, out1, out2) = native_env("embed_macos_native");
    env.lock();
    let lock =
        crate::lock::schema::parse(&std::fs::read_to_string(env.lock_path()).unwrap()).unwrap();
    let natives = &lock.target[0].native;
    let names: Vec<&str> = natives.iter().map(|native| native.name.as_str()).collect();
    assert_eq!(names, ["libout1.dylib", "libout2.dylib"]);
    for (native, source) in natives.iter().zip([&out1, &out2]) {
        assert_eq!(native.required_by, ["tinyb", "tinynat"]);
        assert_eq!(native.sha256, sha256::sha256_file(source).unwrap());
    }
    env.embed_on(&env.toolchain(), EmbedPlatform::MacOs)
        .expect("embedded");
    let lib = env.sidecar().join("lib");
    let closure = env.sidecar().join("closure");
    for dist in ["tinynat", "tinyb"] {
        let ext_deps = deps(&closure.join(dist).join("_ext.so"));
        assert!(
            ext_deps.contains(&"@loader_path/../../lib/libout1.dylib".to_string()),
            "{ext_deps:?}"
        );
    }
    let out1_deps = deps(&lib.join("libout1.dylib"));
    assert_eq!(out1_deps[0], "@rpath/libout1.dylib");
    assert!(
        out1_deps.contains(&"@loader_path/libout2.dylib".to_string()),
        "{out1_deps:?}"
    );
    assert_eq!(deps(&lib.join("libout2.dylib"))[0], "@rpath/libout2.dylib");
    bundle::run_tool(
        "codesign",
        &[OsString::from("-v"), lib.join("libout1.dylib").into()],
    )
    .expect("the vendored native is signed again");
}

/// A native changed after `pycc lock` makes the build's re-derivation
/// differ from the section, so the build is refused naming the library and
/// `pycc lock`, and the previous sidecar is kept.
#[test]
fn a_native_changed_after_the_lock_is_refused() {
    let (env, _, out2) = native_env("embed_macos_native_stale");
    env.lock();
    dylib(&out2, "pycc_fake_out2_changed", &[]);
    env.previous_sidecar();
    let err = env
        .embed_on(&env.toolchain(), EmbedPlatform::MacOs)
        .expect_err("stale");
    assert!(
        err.contains("`[[target.native]]` library `libout2.dylib` differs"),
        "{err}"
    );
    assert!(err.contains("run `pycc lock"), "{err}");
    env.assert_previous_sidecar_intact();
}

/// The relocation vendors a native only when the plan lists it, from bytes
/// that still match its entry. Both are the build's own consistency checks
/// behind the re-derivation, so the test drives `assemble` directly.
#[test]
fn the_relocation_vendors_only_planned_natives_with_their_locked_bytes() {
    let (env, _, _) = native_env("embed_macos_native_plan");
    env.lock();
    let hir = crate::frontend::lock_frontend(&env.entry, InteropCli::default())
        .unwrap_or_else(|_| panic!("the fixture must type-check"));
    let check = crate::lock::build::plan_closure(&env.entry, &hir, HOST)
        .unwrap()
        .unwrap();
    let closure =
        crate::lock::build::payload(&check, &env.lock_probe, EmbedPlatform::MacOs).unwrap();
    let probe = &env.layout.probe;
    let assemble = |plan: &native::NativePlan| {
        let platform = EmbedPlatform::MacOs;
        bundle::assemble(
            probe,
            platform,
            &env.root,
            "app.pycc",
            false,
            Some(&closure),
            plan,
            None,
        )
    };
    let err = assemble(&native::NativePlan::default()).expect_err("unplanned");
    assert!(
        err.contains("`closure/tinyb/_ext.so` (distribution `tinyb`) needs the native library"),
        "{err}"
    );
    assert!(err.contains("do not list"), "{err}");
    let linux = env.toolchain().linux_env();
    let platform = EmbedPlatform::MacOs;
    let mut plan = plan_natives(
        platform,
        probe,
        Some(&closure),
        &linux,
        &WindowsEnv::host(),
        false,
        LibpythonLink::Shared,
    )
    .unwrap();
    plan.natives[0].locked.sha256 = "00".repeat(32);
    let err = assemble(&plan).expect_err("changed bytes");
    assert!(
        err.contains("the native library `libout1.dylib`")
            && err.contains("changed after it was locked"),
        "{err}"
    );
    assert!(!env.sidecar().exists());
}

/// A native reached twice from one distribution is locked once for it,
/// and a native whose `@rpath` dependency resolves nowhere from its own
/// location (it has no `LC_RPATH` entry) is refused by the build naming
/// the native, the distributions that need it and the reason (R2; #1259).
#[test]
fn a_native_whose_rpath_dependency_resolves_nowhere_is_refused() {
    let env = Env::bare("embed_macos_native_relative", "import tinynat\n");
    macho_library(&env.layout);
    let build = env.root.join("build");
    let outside = env.root.join("outside");
    let id = Some("@rpath/libout2.dylib");
    let out2 = image(&build, "libout2.dylib", "-dynamiclib", id, &[], &[]);
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("libout2.dylib"), out2).unwrap();
    let out1 = outside.join("libout1.dylib");
    dylib(&out1, "pycc_fake_out1", &[&outside.join("libout2.dylib")]);
    let ext = image(&build, "tinynat_so", "-bundle", None, &[&out1], &[]);
    let files: [(&str, &[u8]); 3] = [
        ("tinynat/__init__.py", b"X = 1\n"),
        ("tinynat/_a.so", &ext),
        ("tinynat/_b.so", &ext),
    ];
    write_dist(&env.plat, "tinynat", "1.0", &files, &[]);
    env.lock();
    let lock =
        crate::lock::schema::parse(&std::fs::read_to_string(env.lock_path()).unwrap()).unwrap();
    let natives = &lock.target[0].native;
    assert_eq!(natives.len(), 1);
    assert_eq!(natives[0].name, "libout1.dylib");
    assert_eq!(natives[0].required_by, ["tinynat"]);
    let err = env
        .embed_on(&env.toolchain(), EmbedPlatform::MacOs)
        .expect_err("relative");
    assert!(
        err.contains(
            "the native library `lib/libout1.dylib` (required by `tinynat`) depends on \
             `@rpath/libout2.dylib`, which resolves nowhere"
        ),
        "{err}"
    );
    assert!(err.contains("it has no `LC_RPATH` entry"), "{err}");
    assert!(!err.contains("#1259"), "{err}");
    assert!(!env.sidecar().exists());
}
