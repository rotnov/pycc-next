//! The macOS relocation of relative references outside the payload (the
//! pycc.lock decision entry, rule 8; #1259) against real Mach-O images the
//! test builds with `cc`: every bundling case on one build, the refusals,
//! and a site directory inside the prefix. Each lock is written by
//! `pycc lock` itself, so no python3.14 runs.

use super::macos_closure_tests::{deps, image};
use super::*;
use crate::embed::fake_layout::macho_library;

/// The `[[target.native]]` names of `env`'s lock, and whom each is
/// required by.
fn locked_natives(env: &Env) -> Vec<(String, Vec<String>)> {
    let text = std::fs::read_to_string(env.lock_path()).unwrap();
    let lock = crate::lock::schema::parse(&text).unwrap();
    let natives = lock.target[0].native.iter();
    natives
        .map(|native| (native.name.clone(), native.required_by.clone()))
        .collect()
}

/// Builds `<build>/<name>` as a dylib whose install name is `id`, and
/// writes its bytes to `at` too when given.
fn library(build: &Path, name: &str, id: &str, links: &[&Path], rpaths: &[&str]) -> Vec<u8> {
    image(build, name, "-dynamiclib", Some(id), links, rpaths)
}

fn place(at: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, bytes).unwrap();
}

fn assert_has(deps: &[String], expected: &str) {
    assert!(
        deps.iter().any(|dep| dep == expected),
        "{expected}: {deps:?}"
    );
}

/// One build covering every bundling case of #1259:
///
/// 1. `@rpath` whose only match is an absolute rpath outside the payload:
///    a native;
/// 2. an absolute first rpath (a) missing on the host, the payload match
///    rebound, and (b) present, the file there a native;
/// 3. `@loader_path/../..` leaving the payload: a native;
/// 4. a native's `@rpath` dependency through its own `@loader_path` rpath,
///    and its `@loader_path/` dependency: both natives, rewritten;
/// 5. another locked distribution's payload file through `@rpath`: kept;
/// 6. a platlib image whose first `@loader_path` candidate is a purelib
///    distribution's payload path: rebound, not kept;
/// 7. an absolute install name into the payload: rebound, not a native.
#[test]
fn every_relative_bundling_case_relocates_on_one_build() {
    let env = Env::bare(
        "embed_macos_relative",
        "import tinynat\nimport tinyb\nimport tinyp\n",
    );
    macho_library(&env.layout);
    let build = env.root.join("build");
    let outside = env.root.join("outside");
    let plat_nat = env.plat.join("tinynat");

    // Case 1.
    let o1 = library(&build, "libo1.dylib", "@rpath/libo1.dylib", &[], &[]);
    place(&outside.join("rp/libo1.dylib"), &o1);
    let rp = outside.join("rp").display().to_string();
    let c1 = image(
        &build,
        "c1_so",
        "-bundle",
        None,
        &[&build.join("libo1.dylib")],
        &[&rp],
    );
    // Case 2a and 2b.
    let h = library(&build, "libh.dylib", "@rpath/libh.dylib", &[], &[]);
    let c2a_rpaths = ["/pycc-test-nowhere/lib", "@loader_path/.dylibs"];
    let c2a = image(
        &build,
        "c2a_so",
        "-bundle",
        None,
        &[&build.join("libh.dylib")],
        &c2a_rpaths,
    );
    let q = library(&build, "libq.dylib", "@rpath/libq.dylib", &[], &[]);
    place(&outside.join("q/libq.dylib"), &q);
    let q_dir = outside.join("q").display().to_string();
    let c2b_rpaths = [q_dir.as_str(), "@loader_path/.dylibs"];
    let c2b = image(
        &build,
        "c2b_so",
        "-bundle",
        None,
        &[&build.join("libq.dylib")],
        &c2b_rpaths,
    );
    // Case 3: `plat/tinynat/../../outside` is `<root>/outside`.
    let o3_id = "@loader_path/../../outside/libo3.dylib";
    let o3 = library(&build, "libo3.dylib", o3_id, &[], &[]);
    place(&outside.join("libo3.dylib"), &o3);
    let c3 = image(
        &build,
        "c3_so",
        "-bundle",
        None,
        &[&build.join("libo3.dylib")],
        &[],
    );
    // Case 4.
    let n2 = library(&build, "libn2.dylib", "@rpath/libn2.dylib", &[], &[]);
    let n3 = library(&build, "libn3.dylib", "@loader_path/libn3.dylib", &[], &[]);
    let n1_path = outside.join("n/libn1.dylib");
    let n1_id = n1_path.display().to_string();
    let n1_links = [&build.join("libn2.dylib"), &build.join("libn3.dylib")];
    let n1 = library(
        &build,
        "libn1.dylib",
        &n1_id,
        &n1_links.map(|p| p.as_path()),
        &["@loader_path"],
    );
    place(&n1_path, &n1);
    place(&outside.join("n/libn2.dylib"), &n2);
    place(&outside.join("n/libn3.dylib"), &n3);
    let c4 = image(
        &build,
        "c4_so",
        "-bundle",
        None,
        &[&build.join("libn1.dylib")],
        &[],
    );
    // Case 5: `tinyb`'s own library, reached from `tinynat`.
    let b = library(&build, "libb.dylib", "@rpath/libb.dylib", &[], &[]);
    let c5 = image(
        &build,
        "c5_so",
        "-bundle",
        None,
        &[&build.join("libb.dylib")],
        &["@loader_path/../tinyb"],
    );
    // Case 6: `tinyp` (purelib) installs `tinyp/libs.dylib`, and
    // `tinynat` (platlib) its own `.dylibs/libs.dylib`.
    let s = library(&build, "libs.dylib", "@rpath/libs.dylib", &[], &[]);
    let c6_rpaths = ["@loader_path/../tinyp", "@loader_path/.dylibs"];
    let c6 = image(
        &build,
        "c6_so",
        "-bundle",
        None,
        &[&build.join("libs.dylib")],
        &c6_rpaths,
    );
    // Case 7.
    let a_path = plat_nat.join(".dylibs/liba.dylib");
    let a = library(
        &build,
        "liba.dylib",
        &a_path.display().to_string(),
        &[],
        &[],
    );
    let c7 = image(
        &build,
        "c7_so",
        "-bundle",
        None,
        &[&build.join("liba.dylib")],
        &[],
    );

    let nat_files: [(&str, &[u8]); 11] = [
        ("tinynat/__init__.py", b"X = 1\n"),
        ("tinynat/_c1.so", &c1),
        ("tinynat/_c2a.so", &c2a),
        ("tinynat/.dylibs/libh.dylib", &h),
        ("tinynat/_c2b.so", &c2b),
        ("tinynat/.dylibs/libq.dylib", &q),
        ("tinynat/_c3.so", &c3),
        ("tinynat/_c4.so", &c4),
        ("tinynat/_c5.so", &c5),
        ("tinynat/_c6.so", &c6),
        ("tinynat/.dylibs/libs.dylib", &s),
    ];
    let mut nat_files = nat_files.to_vec();
    nat_files.extend([
        ("tinynat/_c7.so", c7.as_slice()),
        ("tinynat/.dylibs/liba.dylib", a.as_slice()),
    ]);
    write_dist(&env.plat, "tinynat", "1.0", &nat_files, &[]);
    let b_files: [(&str, &[u8]); 2] = [("tinyb/__init__.py", b"X = 1\n"), ("tinyb/libb.dylib", &b)];
    write_dist(&env.plat, "tinyb", "1.0", &b_files, &[]);
    let p_files: [(&str, &[u8]); 2] = [("tinyp/__init__.py", b"X = 1\n"), ("tinyp/libs.dylib", &s)];
    write_dist(&env.pure, "tinyp", "1.0", &p_files, &[]);

    env.lock();
    let nat = || vec!["tinynat".to_string()];
    assert_eq!(
        locked_natives(&env),
        [
            ("libn1.dylib".to_string(), nat()),
            ("libn2.dylib".to_string(), nat()),
            ("libn3.dylib".to_string(), nat()),
            ("libo1.dylib".to_string(), nat()),
            ("libo3.dylib".to_string(), nat()),
            ("libq.dylib".to_string(), nat()),
        ]
    );
    env.embed_on(&env.toolchain(), EmbedPlatform::MacOs)
        .expect("embedded");
    let sidecar = env.sidecar();
    let lib = sidecar.join("lib");
    let closure = sidecar.join("closure").join("tinynat");
    let of = |name: &str| deps(&closure.join(name));
    assert_has(&of("_c1.so"), "@loader_path/../../lib/libo1.dylib");
    assert_has(&of("_c2a.so"), "@loader_path/.dylibs/libh.dylib");
    assert_has(&of("_c2b.so"), "@loader_path/../../lib/libq.dylib");
    assert_has(&of("_c3.so"), "@loader_path/../../lib/libo3.dylib");
    assert_has(&of("_c4.so"), "@loader_path/../../lib/libn1.dylib");
    let n1_deps = deps(&lib.join("libn1.dylib"));
    assert_eq!(n1_deps[0], "@rpath/libn1.dylib");
    assert_has(&n1_deps, "@loader_path/libn2.dylib");
    assert_has(&n1_deps, "@loader_path/libn3.dylib");
    assert_has(&of("_c5.so"), "@rpath/libb.dylib");
    assert_has(&of("_c6.so"), "@loader_path/.dylibs/libs.dylib");
    assert_has(&of("_c7.so"), "@loader_path/.dylibs/liba.dylib");
    for kept in ["libh.dylib", "libb.dylib", "libs.dylib", "liba.dylib"] {
        assert!(!lib.join(kept).exists(), "{kept} is not vendored");
    }
    for name in ["_c1.so", "_c6.so"] {
        let path = closure.join(name);
        bundle::run_tool("codesign", &[OsString::from("-v"), path.into()])
            .expect("the rewritten image is signed again");
    }
}

/// Builds `tinynat/_ext.so` linking `links` with `rpaths`, locks it (the
/// derivation never refuses), and returns the build's refusal.
fn refused(tag: &str, links: &[&Path], rpaths: &[&str], env: Env) -> String {
    let build = env.root.join("build");
    let ext = image(&build, &format!("{tag}_so"), "-bundle", None, links, rpaths);
    let files: [(&str, &[u8]); 2] = [
        ("tinynat/__init__.py", b"X = 1\n"),
        ("tinynat/_ext.so", &ext),
    ];
    write_dist(&env.plat, "tinynat", "1.0", &files, &[]);
    env.lock();
    let err = env
        .embed_on(&env.toolchain(), EmbedPlatform::MacOs)
        .expect_err("not relocatable");
    assert!(!env.sidecar().exists(), "{err}");
    assert!(
        err.contains("the locked closure is not relocatable"),
        "{err}"
    );
    err
}

fn refusal_env(tag: &str) -> Env {
    let env = Env::bare(tag, "import tinynat\n");
    macho_library(&env.layout);
    env
}

/// Refusals naming the image, the distribution and the reason: from a
/// closure image, `@executable_path` (R1) and an `@rpath` that resolves
/// nowhere (R2); from a native, `@executable_path` (R1).
#[test]
fn unresolvable_relative_references_are_refused_with_their_reason() {
    let env = refusal_env("embed_macos_relative_r1");
    let build = env.root.join("build");
    library(
        &build,
        "libe.dylib",
        "@executable_path/libe.dylib",
        &[],
        &[],
    );
    let err = refused("r1", &[&build.join("libe.dylib")], &[], env);
    assert!(
        err.contains(
            "`closure/tinynat/_ext.so` (distribution `tinynat`) depends on \
             `@executable_path/libe.dylib`, which is `@executable_path`-relative"
        ),
        "{err}"
    );

    let env = refusal_env("embed_macos_relative_r2");
    let build = env.root.join("build");
    library(&build, "libz9.dylib", "@rpath/libz9.dylib", &[], &[]);
    let err = refused(
        "r2",
        &[&build.join("libz9.dylib")],
        &["@loader_path/none"],
        env,
    );
    assert!(
        err.contains("depends on `@rpath/libz9.dylib`, which resolves nowhere"),
        "{err}"
    );
    assert!(err.contains("/tinynat/none/libz9.dylib`"), "{err}");

    let env = refusal_env("embed_macos_relative_native_r1");
    let build = env.root.join("build");
    let outside = env.root.join("outside").join("libn.dylib");
    library(
        &build,
        "libe.dylib",
        "@executable_path/libe.dylib",
        &[],
        &[],
    );
    let id = outside.display().to_string();
    let n = library(&build, "libn.dylib", &id, &[&build.join("libe.dylib")], &[]);
    place(&outside, &n);
    let err = refused("native_r1", &[&build.join("libn.dylib")], &[], env);
    assert!(
        err.contains(
            "the native library `lib/libn.dylib` (required by `tinynat`) depends on \
             `@executable_path/libe.dylib`, which is `@executable_path`-relative"
        ),
        "{err}"
    );
}

/// With the site directories inside the interpreter's prefix, as a
/// non-venv interpreter has them, an unlocked distribution's library there
/// is a recorded native, not a prefix library vendored unrecorded.
#[test]
fn an_unlocked_site_library_inside_the_prefix_is_a_native() {
    let env = Env::in_prefix("embed_macos_relative_site", "import tinynat\n");
    macho_library(&env.layout);
    let build = env.root.join("build");
    let unlocked = env.plat.join("unlocked").join("libu.dylib");
    let u = library(
        &build,
        "libu.dylib",
        &unlocked.display().to_string(),
        &[],
        &[],
    );
    place(&unlocked, &u);
    let ext = image(
        &build,
        "ext_so",
        "-bundle",
        None,
        &[&build.join("libu.dylib")],
        &[],
    );
    let files: [(&str, &[u8]); 2] = [
        ("tinynat/__init__.py", b"X = 1\n"),
        ("tinynat/_ext.so", &ext),
    ];
    write_dist(&env.plat, "tinynat", "1.0", &files, &[]);
    env.lock();
    assert_eq!(
        locked_natives(&env),
        [("libu.dylib".to_string(), vec!["tinynat".to_string()])]
    );
    env.embed_on(&env.toolchain(), EmbedPlatform::MacOs)
        .expect("embedded");
    let ext_deps = deps(&env.sidecar().join("closure/tinynat/_ext.so"));
    assert_has(&ext_deps, "@loader_path/../../lib/libu.dylib");
}
