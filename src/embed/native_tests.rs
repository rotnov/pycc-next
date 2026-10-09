use super::super::fake_layout::fake_layout;
use super::*;
use pycc_scratch::ScratchDir;

fn file(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("write");
    path
}

/// A name is claimed once per source, case-folded, and never the bundled
/// libpython's.
#[test]
fn names_in_lib_are_claimed_once_per_source() {
    let mut natives = Natives::new("libpython3.14.dylib".to_string());
    let a = Path::new("/one/liba.dylib");
    assert_eq!(natives.claim_name("liba.dylib", a), Ok(false));
    assert_eq!(natives.claim_name("liba.dylib", a), Ok(true));
    let err = natives
        .claim_name("LibA.dylib", Path::new("/two/LibA.dylib"))
        .expect_err("collision");
    assert!(
        err.contains("`/one/liba.dylib` and `/two/LibA.dylib`"),
        "{err}"
    );
    assert!(err.contains("(as `liba.dylib` and `LibA.dylib`)"), "{err}");
    let err = natives
        .claim_name("LIBPYTHON3.14.dylib", Path::new("/x/LIBPYTHON3.14.dylib"))
        .expect_err("bundled");
    assert!(err.contains("the bundled libpython already has"), "{err}");
}

/// Each native is hashed once and gathers every distribution that needs
/// it; the result is sorted by name with sorted owners.
#[test]
fn natives_gather_their_owners_and_sort_by_name() {
    let dir = ScratchDir::new("native_owners").expect("scratch");
    let z = file(&dir, "libz.so.1", "z");
    let a = file(&dir, "liba.so.1", "a");
    let mut natives = Natives::new("libpython3.14.so.1.0".to_string());
    assert_eq!(natives.add(&z, "libz.so.1", "zpkg"), Ok(true));
    assert_eq!(natives.add(&z, "libz.so.1", "apkg"), Ok(true));
    assert_eq!(natives.add(&z, "libz.so.1", "zpkg"), Ok(false));
    assert_eq!(natives.add(&a, "liba.so.1", "zpkg"), Ok(true));
    let finished = natives.finish();
    let names: Vec<&str> = finished.iter().map(|n| n.locked.name.as_str()).collect();
    assert_eq!(names, ["liba.so.1", "libz.so.1"]);
    assert_eq!(finished[1].locked.required_by, ["apkg", "zpkg"]);
    assert_eq!(
        finished[1].locked.sha256,
        super::super::sha256::sha256_hex(b"z")
    );
    assert_eq!(finished[1].source, z);
}

#[test]
fn an_unreadable_native_is_refused_by_name() {
    let dir = ScratchDir::new("native_unreadable").expect("scratch");
    let mut natives = Natives::new("libpython3.14.so.1.0".to_string());
    let err = natives
        .add(&dir.join("libgone.so"), "libgone.so", "pkg")
        .expect_err("unreadable");
    assert!(
        err.contains("(for distribution `pkg`), which cannot be read"),
        "{err}"
    );
}

#[test]
fn the_small_helpers_read_heads_resolve_paths_and_name_files() {
    let dir = ScratchDir::new("native_helpers").expect("scratch");
    let long = file(&dir, "long", "0123456789");
    assert_eq!(read_head(&long), Ok(b"01234567".to_vec()));
    let short = file(&dir, "short", "ab");
    assert_eq!(read_head(&short), Ok(b"ab".to_vec()));
    let err = read_head(&dir.join("gone")).expect_err("missing");
    assert!(err.contains("could not read"), "{err}");
    let gone = dir.join("gone");
    assert_eq!(resolved(&gone), gone);
    assert_eq!(resolved(&long), std::fs::canonicalize(&long).unwrap());
    assert_eq!(file_name(Path::new("/a/libx.dylib")), "libx.dylib");
    assert_eq!(file_name(Path::new("/")), "");
}

/// macOS derives nothing without a closure, or from a closure with no
/// Mach-O image, and so never runs `otool` for it; an unreadable closure
/// file is refused.
#[test]
fn a_macos_closure_without_images_has_no_natives() {
    let dir = ScratchDir::new("native_macos_empty").expect("scratch");
    let layout = fake_layout(&dir);
    let env = LinuxEnv::host();
    let platform = EmbedPlatform::MacOs;
    let plan = plan_natives(
        platform,
        &layout.probe,
        None,
        &env,
        &WindowsEnv::host(),
        true,
        LibpythonLink::Shared,
    )
    .expect("empty");
    assert!(plan.natives.is_empty() && plan.linux_vendor.is_empty());
    let source = file(&dir, "mod.py", "X = 1\n");
    let files = vec![ClosureFile {
        rel: "p/mod.py".to_string(),
        source,
        digest: String::new(),
        package: "p".to_string(),
    }];
    let closure = LockedClosure::of_files(files);
    let plan = plan_natives(
        platform,
        &layout.probe,
        Some(&closure),
        &env,
        &WindowsEnv::host(),
        false,
        LibpythonLink::Shared,
    );
    assert!(plan.expect("no images").natives.is_empty());
    let files = vec![ClosureFile {
        rel: "p/gone.so".to_string(),
        source: dir.join("gone.so"),
        digest: String::new(),
        package: "p".to_string(),
    }];
    let closure = LockedClosure::of_files(files);
    let err = plan_natives(
        platform,
        &layout.probe,
        Some(&closure),
        &env,
        &WindowsEnv::host(),
        false,
        LibpythonLink::Shared,
    );
    assert!(err.expect_err("unreadable").contains("could not read"));
}

/// The production probe: an on-disk file by its canonical path, a system
/// library the dyld shared cache serves by its own path, and nothing for
/// a made-up system path, an absent file or a path no C string can hold.
#[cfg(target_os = "macos")]
#[test]
fn the_host_probe_finds_files_and_the_dyld_shared_cache() {
    let dir = ScratchDir::new("native_on_host").expect("scratch");
    let lib = file(&dir, "libx.dylib", "x");
    assert_eq!(on_host(&lib), Some(std::fs::canonicalize(&lib).unwrap()));
    assert_eq!(on_host(&dir.join("absent.dylib")), None);
    let cached = Path::new("/usr/lib/libz.1.dylib");
    assert!(on_host(cached).is_some());
    assert!(in_shared_cache(cached));
    let made_up = Path::new("/usr/lib/swift/libmyhelper.dylib");
    assert_eq!(on_host(made_up), None);
    assert!(!in_shared_cache(made_up));
    assert!(!in_shared_cache(Path::new("/usr/lib/lib\0z.dylib")));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn there_is_no_dyld_shared_cache_off_macos() {
    assert!(!in_shared_cache(Path::new("/usr/lib/libz.1.dylib")));
}
