//! Every arm of the host classifier over an injected probe: a scanned
//! purelib `/s/pure` and platlib `/s/plat`, an interpreter prefix `/p`
//! with its own `site-packages`, and a closure image `pkg/ext.so` in
//! platlib.

use super::*;

const LIBPYTHON: &str = "/p/lib/libpython3.14.dylib";

fn context() -> HostContext {
    let payload = [
        ("/s/plat/pkg/ext.so", "pkg/ext.so"),
        ("/s/plat/pkg/.dylibs/libx.dylib", "pkg/.dylibs/libx.dylib"),
        ("/s/plat/pkg/.dylibs/liby.dylib", "pkg/.dylibs/liby.dylib"),
        ("/s/pure/other/liby.dylib", "other/liby.dylib"),
    ];
    HostContext {
        prefix: PathBuf::from("/p"),
        bundle_lib: vec![
            PathBuf::from("@rpath/libpython3.14.dylib"),
            PathBuf::from(LIBPYTHON),
        ],
        scanned_sites: vec![PathBuf::from("/s/pure"), PathBuf::from("/s/plat")],
        stdlib_sites: PathBuf::from("/p/lib/python3.14/site-packages"),
        payload: payload
            .into_iter()
            .map(|(source, rel)| (PathBuf::from(source), rel.to_string()))
            .collect(),
    }
}

/// The host's files: every payload file, a symlink into the payload, an
/// unlocked distribution's library in platlib and in the prefix's
/// `site-packages`, a prefix library, libpython, a library outside
/// everything, and a system library served by the shared cache.
fn probe(path: &Path) -> Option<PathBuf> {
    let link = Path::new("/s/plat/pkg/link.dylib");
    if path == link {
        return Some(PathBuf::from("/s/plat/pkg/.dylibs/libx.dylib"));
    }
    let files = [
        "/s/plat/pkg/ext.so",
        "/s/plat/pkg/.dylibs/libx.dylib",
        "/s/plat/pkg/.dylibs/liby.dylib",
        "/s/pure/other/liby.dylib",
        "/s/plat/unlocked/libu.dylib",
        "/p/lib/python3.14/site-packages/u/libs.dylib",
        "/p/lib/libffi.8.dylib",
        LIBPYTHON,
        "/opt/out/libo.dylib",
        "/usr/lib/libz.1.dylib",
    ];
    files
        .iter()
        .any(|file| Path::new(file) == path)
        .then(|| path.to_path_buf())
}

fn classify(dep: &str, kind: HostKind, own_id: Option<&str>, rpaths: &[&str]) -> MachoDep {
    let rpaths: Vec<String> = rpaths.iter().map(|rpath| rpath.to_string()).collect();
    let (sidecar_rel, source_dir) = match kind {
        HostKind::Closure => ("closure/pkg/ext.so", Path::new("/s/plat/pkg")),
        HostKind::Native => ("lib/libo.dylib", Path::new("/opt/out")),
    };
    let image = HostImage {
        sidecar_rel,
        source_dir,
        own_id,
        rpaths: &rpaths,
        kind,
    };
    classify_on_host(dep, &image, &context(), &probe)
}

fn closure(dep: &str, rpaths: &[&str]) -> MachoDep {
    classify(dep, HostKind::Closure, None, rpaths)
}

fn rebind(to: &str) -> MachoDep {
    MachoDep::Rebind(to.to_string())
}

fn refusal(class: MachoDep) -> String {
    match class {
        MachoDep::RefuseWith(reason) => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn the_own_id_and_absolute_system_libraries_are_kept() {
    let own = Some("@rpath/libself.dylib");
    let kind = HostKind::Closure;
    assert_eq!(
        classify("@rpath/libself.dylib", kind, own, &[]),
        MachoDep::Keep
    );
    assert_eq!(closure("/usr/lib/libSystem.B.dylib", &[]), MachoDep::Keep);
    assert_eq!(
        closure("/System/Library/Frameworks/CoreFoundation", &[]),
        MachoDep::Keep
    );
}

/// By spelling before resolution (an `@rpath` id with no `LC_RPATH`
/// entry), and by the path a relative reference resolves to.
#[test]
fn the_source_libpython_is_rewritten_by_spelling_and_by_resolved_path() {
    for dep in ["@rpath/libpython3.14.dylib", LIBPYTHON] {
        assert_eq!(closure(dep, &[]), MachoDep::RewriteToBundled);
    }
    assert_eq!(
        closure("@loader_path/../../../p/lib/libpython3.14.dylib", &[]),
        MachoDep::RewriteToBundled
    );
}

/// Rule K: the relative spelling stays when it resolves identically in
/// the sidecar, including through a missing earlier `@loader_path`
/// candidate and through `@loader_path` itself.
#[test]
fn a_loader_relative_reference_to_its_payload_path_is_kept() {
    assert_eq!(
        closure("@loader_path/.dylibs/libx.dylib", &[]),
        MachoDep::Keep
    );
    let rpaths = ["@loader_path/none", "@loader_path/.dylibs"];
    assert_eq!(closure("@rpath/libx.dylib", &rpaths), MachoDep::Keep);
    assert_eq!(
        closure("@rpath/.dylibs/libx.dylib", &["@loader_path"]),
        MachoDep::Keep
    );
    // `..` that stays inside the site directory keeps too.
    assert_eq!(
        closure("@loader_path/../pkg/.dylibs/libx.dylib", &[]),
        MachoDep::Keep
    );
    let rpaths = ["@loader_path/../pkg/./.dylibs"];
    assert_eq!(closure("@rpath/libx.dylib", &rpaths), MachoDep::Keep);
}

/// Each of rule K's conditions, failing alone, rebinds the payload hit to
/// an explicit path instead.
#[test]
fn a_payload_hit_that_rule_k_does_not_keep_is_rebound() {
    let libx = rebind("@loader_path/.dylibs/libx.dylib");
    // A candidate before the match that is not `@loader_path`-relative.
    let rpaths = ["/pycc-nowhere/lib", "@loader_path/.dylibs"];
    assert_eq!(closure("@rpath/libx.dylib", &rpaths), libx);
    // An earlier candidate that climbs above the site directory.
    let rpaths = ["@loader_path/../../../elsewhere", "@loader_path/.dylibs"];
    assert_eq!(closure("@rpath/libx.dylib", &rpaths), libx);
    // A match found under another name than its payload path.
    assert_eq!(closure("@loader_path/link.dylib", &[]), libx);
    // A match whose spelling climbs above the site directory and comes
    // back in: `closure/` replaces the site's name, so the spelling would
    // not resolve in the sidecar, in the reference or in an rpath entry.
    let climbing = "@loader_path/../../plat/pkg/.dylibs/libx.dylib";
    assert_eq!(closure(climbing, &[]), libx);
    let rpaths = ["@loader_path/../../plat/pkg/.dylibs"];
    assert_eq!(closure("@rpath/libx.dylib", &rpaths), libx);
    let climbing_tail = "@rpath/../../plat/pkg/.dylibs/libx.dylib";
    assert_eq!(closure(climbing_tail, &["@loader_path"]), libx);
    // An earlier candidate at another site directory's payload path,
    // which `closure/` would hold and dyld would load first.
    let rpaths = ["@loader_path/../other", "@loader_path/.dylibs"];
    assert_eq!(
        closure("@rpath/liby.dylib", &rpaths),
        rebind("@loader_path/.dylibs/liby.dylib")
    );
}

/// Another locked distribution's file, an absolute install name into the
/// payload, and a native's reference into the payload are all rebound to
/// the closure copy, never vendored.
#[test]
fn other_payload_files_are_rebound_to_their_closure_copies() {
    assert_eq!(
        closure("@rpath/liby.dylib", &["@loader_path/../../pure/other"]),
        rebind("@loader_path/../other/liby.dylib")
    );
    assert_eq!(closure("/s/plat/pkg/.dylibs/libx.dylib", &[]), libx());
    let dep = "@loader_path/../../s/plat/pkg/.dylibs/libx.dylib";
    assert_eq!(
        classify(dep, HostKind::Native, Some("@rpath/libo.dylib"), &[]),
        rebind("@loader_path/../closure/pkg/.dylibs/libx.dylib")
    );
}

fn libx() -> MachoDep {
    rebind("@loader_path/.dylibs/libx.dylib")
}

#[test]
fn a_relatively_named_system_library_is_rebound_to_its_absolute_path() {
    let rpaths = ["/usr/lib/swift", "/usr/lib"];
    assert_eq!(
        closure("@rpath/libz.1.dylib", &rpaths),
        rebind("/usr/lib/libz.1.dylib")
    );
}

/// An unlocked distribution's file is a native even under the prefix; a
/// prefix library is vendored; anything else, found or not, is a native.
#[test]
fn site_files_are_natives_prefix_files_vendored_and_the_rest_natives() {
    let unlocked = PathBuf::from("/s/plat/unlocked/libu.dylib");
    assert_eq!(
        closure("@loader_path/../unlocked/libu.dylib", &[]),
        MachoDep::VendorNative(unlocked)
    );
    let stdlib_site = "/p/lib/python3.14/site-packages/u/libs.dylib";
    assert_eq!(
        closure(stdlib_site, &[]),
        MachoDep::VendorNative(PathBuf::from(stdlib_site))
    );
    let ffi = PathBuf::from("/p/lib/libffi.8.dylib");
    assert_eq!(
        closure("@rpath/libffi.8.dylib", &["/p/lib"]),
        MachoDep::Vendor(ffi.clone())
    );
    assert_eq!(closure("/p/lib/libffi.8.dylib", &[]), MachoDep::Vendor(ffi));
    let out = PathBuf::from("/opt/out/libo.dylib");
    assert_eq!(
        closure("@rpath/libo.dylib", &["/opt/out"]),
        MachoDep::VendorNative(out)
    );
    let gone = PathBuf::from("/opt/gone.dylib");
    assert_eq!(
        closure("/opt/gone.dylib", &[]),
        MachoDep::VendorNative(gone)
    );
}

/// R1: `@executable_path`, as the reference or as an rpath reached first.
#[test]
fn an_executable_path_reference_is_refused() {
    let reason = refusal(closure("@executable_path/libz.dylib", &[]));
    assert!(reason.contains("`@executable_path`-relative"), "{reason}");
    assert!(reason.contains("embedded executable"), "{reason}");
    let rpaths = ["@executable_path/../lib", "@loader_path/.dylibs"];
    let reason = refusal(closure("@rpath/libx.dylib", &rpaths));
    assert!(
        reason.contains("`LC_RPATH` entry `@executable_path/../lib`, which names the program"),
        "{reason}"
    );
}

/// R2: nothing found from the image's own location and rpaths.
#[test]
fn a_reference_that_resolves_nowhere_is_refused_naming_what_was_searched() {
    let reason = refusal(closure("@rpath/libx.dylib", &[]));
    assert!(reason.contains("resolves nowhere"), "{reason}");
    assert!(reason.contains("it has no `LC_RPATH` entry"), "{reason}");
    let reason = refusal(closure("@rpath/libq.dylib", &["@loader_path/.dylibs"]));
    assert!(
        reason.contains("searched `/s/plat/pkg/.dylibs/libq.dylib`"),
        "{reason}"
    );
    let reason = refusal(closure("@loader_path/libq.dylib", &[]));
    assert!(reason.contains("`/s/plat/pkg/libq.dylib`"), "{reason}");
}

/// R3: any other form of reference or rpath entry.
#[test]
fn an_unknown_form_of_reference_or_rpath_is_refused() {
    for dep in ["@foo/libx.dylib", "libx.dylib", "@loader_pathx/libx.dylib"] {
        let reason = refusal(closure(dep, &[]));
        assert!(
            reason.contains("is neither absolute nor an `@rpath`"),
            "{reason}"
        );
    }
    for rpath in ["@foo/lib", "@loader_pathx"] {
        let reason = refusal(closure("@rpath/libx.dylib", &[rpath]));
        assert!(
            reason.contains(&format!("entry `{rpath}`, which is neither absolute nor")),
            "{reason}"
        );
    }
}

/// `Path::components` already drops an interior `.`; only a leading one
/// of a relative path reaches the `CurDir` arm.
#[test]
fn normalize_drops_dot_and_dot_dot_lexically() {
    assert_eq!(normalize(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
    assert_eq!(normalize(Path::new("/../a")), PathBuf::from("/a"));
    assert_eq!(normalize(Path::new("./a/../b")), PathBuf::from("b"));
}

/// R1 and R3 on an `LC_RPATH` entry apply only when the search reaches it:
/// an entry after the match is never consulted, as dyld never does.
#[test]
fn a_bad_rpath_entry_after_the_match_is_not_reached() {
    for bad in ["@executable_path/../lib", "@foo/lib"] {
        let rpaths = ["@loader_path/.dylibs", bad];
        assert_eq!(closure("@rpath/libx.dylib", &rpaths), MachoDep::Keep);
        let rpaths = [bad, "@loader_path/.dylibs"];
        assert!(matches!(
            closure("@rpath/libx.dylib", &rpaths),
            MachoDep::RefuseWith(_)
        ));
    }
}

/// The walk yields the site-relative path while it stays inside the site
/// directory (a leading `./` reaches the `CurDir` arm; interior ones are
/// dropped by `Path::components`), and nothing once it climbs out.
#[test]
fn within_site_walks_the_spelling_and_refuses_to_leave_the_site() {
    let within = |start: &str, spelled: &str| within_site(Path::new(start), Path::new(spelled));
    assert_eq!(
        within("pkg", "./.dylibs/libx.dylib").as_deref(),
        Some("pkg/.dylibs/libx.dylib")
    );
    assert_eq!(
        within("pkg", "../pkg/libx.dylib").as_deref(),
        Some("pkg/libx.dylib")
    );
    assert_eq!(within("pkg", "../../plat/pkg/libx.dylib"), None);
}
