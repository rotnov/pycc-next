use super::*;
use crate::lock::fixture::{append_record, record_hash, write_dist};
use crate::lock::marker::tests::env;
use pycc_scratch::ScratchDir;

struct Env {
    _dir: ScratchDir,
    pure: PathBuf,
    plat: PathBuf,
}

impl Env {
    fn new(tag: &str) -> Self {
        let dir = ScratchDir::new(tag).unwrap();
        let pure = std::fs::canonicalize(&*dir).unwrap().join("pure");
        let plat = std::fs::canonicalize(&*dir).unwrap().join("plat");
        std::fs::create_dir_all(&pure).unwrap();
        std::fs::create_dir_all(&plat).unwrap();
        Self {
            _dir: dir,
            pure,
            plat,
        }
    }

    /// Resolves for a POSIX host, whatever host the test runs on.
    fn resolve(&self, roots: &[&str]) -> Result<Vec<ResolvedPackage>, String> {
        self.resolve_on(roots, EmbedPlatform::Linux)
    }

    fn resolve_on(
        &self,
        roots: &[&str],
        platform: EmbedPlatform,
    ) -> Result<Vec<ResolvedPackage>, String> {
        let sites = scanned_sites(&self.pure, &self.plat)?;
        let roots = roots.iter().map(|r| r.to_string()).collect();
        resolve(&sites, &roots, &BTreeSet::new(), &env(), platform)
    }

    /// Resolves `required` and `optional` roots (#1290) for a POSIX host.
    fn resolve_split(
        &self,
        required: &[&str],
        optional: &[&str],
    ) -> Result<Vec<ResolvedPackage>, String> {
        let sites = scanned_sites(&self.pure, &self.plat)?;
        let set = |roots: &[&str]| roots.iter().map(|r| r.to_string()).collect();
        resolve(
            &sites,
            &set(required),
            &set(optional),
            &env(),
            EmbedPlatform::Linux,
        )
    }

    fn names(&self, roots: &[&str]) -> Vec<String> {
        self.resolve(roots)
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect()
    }

    fn refusal(&self, roots: &[&str]) -> String {
        self.resolve(roots).expect_err("the lock must be refused")
    }
}

fn tiny(site: &Path, requires: &[&str]) -> PathBuf {
    write_dist(
        site,
        "tinypkg",
        "1.0",
        &[
            ("tinypkg/__init__.py", b"X = 1\n"),
            ("tinypkg/sub.py", b"Y = 2\n"),
        ],
        requires,
    )
}

#[test]
fn a_root_and_its_dependency_lock_with_digests_and_edges() {
    let env = Env::new("lock_resolve_basic");
    tiny(&env.pure, &["tinydep>=1"]);
    write_dist(
        &env.plat,
        "tinydep",
        "2.0",
        &[("tinydep.py", b"Z = 3\n")],
        &[],
    );
    let packages = env.resolve(&["tinypkg"]).unwrap();
    assert_eq!(packages.len(), 2);
    let (dep, pkg) = (&packages[0], &packages[1]);
    assert_eq!(
        (dep.name.as_str(), dep.version.as_str(), dep.site),
        ("tinydep", "2.0", SiteKind::Platlib)
    );
    assert_eq!(dep.files, 2, "tinydep.py and METADATA");
    assert_eq!(
        (pkg.name.as_str(), pkg.site, pkg.files),
        ("tinypkg", SiteKind::Purelib, 3)
    );
    assert_eq!(pkg.requires, ["tinydep"]);
    assert!(dep.requires.is_empty());
}

#[test]
fn the_tree_digest_is_the_documented_formula_and_ignores_record_order() {
    let env = Env::new("lock_resolve_digest");
    let dist = write_dist(
        &env.pure,
        "two",
        "1",
        &[("two/a.py", b"a"), ("two/b.py", b"b")],
        &[],
    );
    let metadata = std::fs::read(dist.join("METADATA")).unwrap();
    let line =
        |path: &str, bytes: &[u8]| format!("{path}\0{}\n", crate::embed::sha256::sha256_hex(bytes));
    let expected = crate::embed::sha256::sha256_hex(
        format!(
            "{}{}{}",
            line("two-1.dist-info/METADATA", &metadata),
            line("two/a.py", b"a"),
            line("two/b.py", b"b")
        )
        .as_bytes(),
    );
    assert_eq!(env.resolve(&["two"]).unwrap()[0].tree_sha256, expected);
    // Reversing RECORD's rows changes nothing.
    let record = std::fs::read_to_string(dist.join("RECORD")).unwrap();
    let reversed: String = record.lines().rev().map(|l| format!("{l}\n")).collect();
    std::fs::write(dist.join("RECORD"), reversed).unwrap();
    // Installer files and caches listed in RECORD are excluded.
    append_record(&dist, "two-1.dist-info/REQUESTED,sha256=bogus,0");
    append_record(&dist, "two-1.dist-info/direct_url.json,,");
    append_record(&dist, "two/__pycache__/a.cpython-314.pyc,,");
    append_record(&dist, "../../../bin/tool,sha256=bogus,1");
    assert_eq!(env.resolve(&["two"]).unwrap()[0].tree_sha256, expected);
}

#[test]
fn a_mutated_payload_byte_is_refused_naming_the_file() {
    let env = Env::new("lock_resolve_mutated");
    tiny(&env.pure, &[]);
    std::fs::write(env.pure.join("tinypkg/sub.py"), b"Y = 9\n").unwrap();
    let err = env.refusal(&["tinypkg"]);
    assert!(
        err.contains("tinypkg/sub.py") && err.contains("does not match its RECORD hash"),
        "{err}"
    );
}

#[test]
fn a_namespace_root_owned_by_two_distributions_locks_both() {
    let env = Env::new("lock_resolve_namespace");
    write_dist(&env.pure, "ns-a", "1", &[("ns/a.py", b"a")], &[]);
    write_dist(&env.pure, "ns-b", "1", &[("ns/b.py", b"b")], &[]);
    assert_eq!(env.names(&["ns"]), ["ns-a", "ns-b"]);
}

#[test]
fn markers_and_extras_select_requirements() {
    let env = Env::new("lock_resolve_markers");
    tiny(
        &env.pure,
        &[
            "unused ; sys_platform == 'win32'",
            "unused2 ; extra == 'extra1'",
            "b[x]",
            "always ; extra != 'x'",
        ],
    );
    write_dist(
        &env.pure,
        "b",
        "1",
        &[("b.py", b"")],
        &["bx ; extra == 'x'", "by ; extra == 'y'"],
    );
    write_dist(&env.pure, "bx", "1", &[("bx.py", b"")], &[]);
    write_dist(&env.pure, "always", "1", &[("always.py", b"")], &[]);
    assert_eq!(env.names(&["tinypkg"]), ["always", "b", "bx", "tinypkg"]);
}

#[test]
fn two_requested_extras_each_pull_their_requirements_and_a_later_extra_requeues() {
    let env = Env::new("lock_resolve_extras");
    tiny(&env.pure, &["b[x]", "c"]);
    write_dist(&env.pure, "c", "1", &[("c.py", b"")], &["b[y]", "c[z]"]);
    write_dist(
        &env.pure,
        "b",
        "1",
        &[("b.py", b"")],
        &["bx ; extra == 'x'", "by ; extra == 'y'"],
    );
    write_dist(&env.pure, "bx", "1", &[("bx.py", b"")], &[]);
    write_dist(&env.pure, "by", "1", &[("by.py", b"")], &[]);
    let packages = env.resolve(&["tinypkg"]).unwrap();
    let names: Vec<_> = packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["b", "bx", "by", "c", "tinypkg"]);
    let c = packages.iter().find(|p| p.name == "c").unwrap();
    assert_eq!(c.requires, ["b"], "no self-edge for c[z]");
}

#[test]
fn distributions_outside_the_closure_are_never_parsed() {
    let env = Env::new("lock_resolve_scope");
    tiny(&env.pure, &[]);
    // Homebrew shape: no RECORD.
    std::fs::create_dir(env.pure.join("brewed-1.0.dist-info")).unwrap();
    // Unparsable RECORD, unparsable marker, garbage METADATA, a mismatch.
    let bad = write_dist(&env.pure, "badrecord", "1", &[("badrecord.py", b"")], &[]);
    std::fs::write(bad.join("RECORD"), "\"unterminated").unwrap();
    let marker = write_dist(
        &env.pure,
        "badmarker",
        "1",
        &[("badmarker.py", b"")],
        &["x ; bogus == '1'"],
    );
    std::fs::write(marker.join("METADATA"), "garbage").unwrap();
    write_dist(&env.pure, "mismatch", "1", &[("mismatch.py", b"a")], &[]);
    std::fs::write(env.pure.join("mismatch.py"), b"b").unwrap();
    assert_eq!(env.names(&["tinypkg"]), ["tinypkg"]);
}

#[test]
fn an_unowned_root_is_refused() {
    let env = Env::new("lock_resolve_unowned");
    tiny(&env.pure, &[]);
    let err = env.refusal(&["missing"]);
    assert!(
        err.contains("no installed distribution owns import root `missing`"),
        "{err}"
    );
}

#[test]
fn an_uninstalled_requirement_is_refused_naming_it() {
    let env = Env::new("lock_resolve_uninstalled");
    tiny(&env.pure, &["ghost>=1"]);
    std::fs::create_dir(env.pure.join("ghost-1.0.egg-info")).unwrap();
    let err = env.refusal(&["tinypkg"]);
    assert!(
        err.contains("requires `ghost>=1`") && err.contains("`ghost` is not installed"),
        "{err}"
    );
}

#[test]
fn closure_members_need_metadata_and_record() {
    let env = Env::new("lock_resolve_member_files");
    tiny(&env.pure, &["dep"]);
    let dep = write_dist(&env.pure, "dep", "1", &[("dep.py", b"")], &[]);
    std::fs::remove_file(dep.join("RECORD")).unwrap();
    // Messages show native paths; compare with `/` on every host.
    let err = env.refusal(&["tinypkg"]).replace('\\', "/");
    assert!(
        err.contains("dep-1.dist-info/RECORD") && err.contains("build from a venv"),
        "{err}"
    );
    std::fs::write(dep.join("RECORD"), "").unwrap();
    std::fs::remove_file(dep.join("METADATA")).unwrap();
    let err = env.refusal(&["tinypkg"]).replace('\\', "/");
    assert!(
        err.contains("dep-1.dist-info/METADATA") && err.contains("build from a venv"),
        "{err}"
    );
    std::fs::write(dep.join("METADATA"), "Name: other\nVersion: 1\n").unwrap();
    let err = env.refusal(&["tinypkg"]).replace('\\', "/");
    assert!(err.contains("`Name: other`"), "{err}");
}

#[test]
fn a_bad_requirement_or_marker_in_the_closure_is_refused() {
    let env = Env::new("lock_resolve_bad_requirement");
    let dist = tiny(&env.pure, &["x ; python_version >= '3.8.0rc1'"]);
    let err = env.refusal(&["tinypkg"]);
    assert!(
        err.contains("distribution `tinypkg`") && err.contains("plain release"),
        "{err}"
    );
    let meta = std::fs::read_to_string(dist.join("METADATA")).unwrap();
    std::fs::write(
        dist.join("METADATA"),
        meta.replace("x ; python_version >= '3.8.0rc1'", "[bad"),
    )
    .unwrap();
    // The METADATA no longer matches RECORD, but requirements are read first.
    let err = env.refusal(&["tinypkg"]);
    assert!(
        err.contains("distribution `tinypkg`") && err.contains("distribution name"),
        "{err}"
    );
}

#[test]
fn an_editable_install_is_refused_and_a_plain_direct_url_is_not() {
    let env = Env::new("lock_resolve_editable");
    let dist = tiny(&env.pure, &[]);
    std::fs::write(
        dist.join("direct_url.json"),
        r#"{"url": "file:///x", "dir_info": {}}"#,
    )
    .unwrap();
    env.resolve(&["tinypkg"]).unwrap();
    std::fs::write(
        dist.join("direct_url.json"),
        r#"{"url": "file:///x", "dir_info": {"editable": true}}"#,
    )
    .unwrap();
    assert!(env.refusal(&["tinypkg"]).contains("editable install"));
    std::fs::write(dist.join("direct_url.json"), "{").unwrap();
    assert!(env.refusal(&["tinypkg"]).contains("cannot parse"));
}

#[test]
fn a_top_level_pth_file_is_refused() {
    let env = Env::new("lock_resolve_pth");
    let dist = tiny(&env.pure, &[]);
    std::fs::write(env.pure.join("__editable__.tinypkg.pth"), b"/src").unwrap();
    append_record(
        &dist,
        &format!("__editable__.tinypkg.pth,{},4", record_hash(b"/src")),
    );
    assert!(env.refusal(&["tinypkg"]).contains("top-level `.pth`"));
}

#[test]
fn a_duplicate_distribution_name_is_refused_when_reached() {
    let env = Env::new("lock_resolve_duplicate");
    tiny(&env.pure, &[]);
    write_dist(
        &env.plat,
        "TinyPkg",
        "2.0",
        &[("tinypkg_other.py", b"")],
        &[],
    );
    let err = env.refusal(&["tinypkg"]);
    assert!(err.contains("installed more than once"), "{err}");
    let env = Env::new("lock_resolve_duplicate_owner");
    tiny(&env.pure, &[]);
    write_dist(
        &env.plat,
        "tinypkg",
        "2.0",
        &[("tinypkg/other.py", b"")],
        &[],
    );
    assert!(
        env.refusal(&["tinypkg"])
            .contains("installed more than once")
    );
}

#[test]
fn payload_path_refusals() {
    let cases: [(&str, &str, &str); 5] = [
        ("/abs/x.py,sha256=AA,1", "absolute path", "abs"),
        (
            "../plat/tinypkg_split.py,sha256=AA,1",
            "other scanned site",
            "split",
        ),
        (
            "tinypkg/../tinypkg/sub.py,sha256=AA,1",
            "contains `..`",
            "dotdot",
        ),
        ("tinypkg/nohash.py,,1", "no sha256 hash", "nohash"),
        ("tinypkg/sha512.py,sha512=AA,1", "no sha256 hash", "sha512"),
    ];
    for (line, why, tag) in cases {
        let env = Env::new(&format!("lock_resolve_payload_{tag}"));
        let dist = tiny(&env.pure, &[]);
        append_record(&dist, line);
        let err = env.refusal(&["tinypkg"]);
        assert!(err.contains(why), "{line}: {err}");
    }
    let env = Env::new("lock_resolve_payload_more");
    let dist = tiny(&env.pure, &[]);
    let record = std::fs::read_to_string(dist.join("RECORD")).unwrap();
    append_record(&dist, record.lines().next().unwrap());
    assert!(env.refusal(&["tinypkg"]).contains("listed twice"));
    std::fs::write(dist.join("RECORD"), &record).unwrap();
    append_record(&dist, "tinypkg/bad.py,sha256=!!,1");
    assert!(env.refusal(&["tinypkg"]).contains("malformed sha256"));
    std::fs::write(dist.join("RECORD"), &record).unwrap();
    append_record(&dist, &format!("tinypkg/gone.py,{},1", record_hash(b"")));
    assert!(env.refusal(&["tinypkg"]).contains("missing on disk"));
}

#[cfg(unix)]
#[test]
fn symlinks_in_a_locked_package_are_refused() {
    let env = Env::new("lock_resolve_symlink");
    tiny(&env.pure, &[]);
    std::fs::write(env.pure.join("elsewhere.py"), b"").unwrap();
    std::os::unix::fs::symlink(
        env.pure.join("elsewhere.py"),
        env.pure.join("tinypkg/link.py"),
    )
    .unwrap();
    let err = env.refusal(&["tinypkg"]);
    assert!(
        err.contains("tinypkg/link.py") && err.contains("symbolic link"),
        "{err}"
    );
    // A symlinked payload file of a non-root closure member.
    let env = Env::new("lock_resolve_symlink_payload");
    tiny(&env.pure, &["dep"]);
    let dep = write_dist(&env.pure, "dep", "1", &[("dep/real.py", b"r")], &[]);
    std::os::unix::fs::symlink(env.pure.join("dep/real.py"), env.pure.join("dep/alias.py"))
        .unwrap();
    append_record(&dep, &format!("dep/alias.py,{},1", record_hash(b"r")));
    assert!(env.refusal(&["tinypkg"]).contains("symbolic link"));
    // A site directory reached through a symlink scans once.
    let env = Env::new("lock_resolve_symlinked_site");
    tiny(&env.pure, &[]);
    std::fs::remove_dir(&env.plat).unwrap();
    std::os::unix::fs::symlink(&env.pure, &env.plat).unwrap();
    assert_eq!(env.names(&["tinypkg"]), ["tinypkg"]);
}

#[test]
fn on_disk_files_under_a_root_must_be_recorded_by_location() {
    let env = Env::new("lock_resolve_cover_extra");
    tiny(&env.pure, &[]);
    std::fs::write(env.pure.join("tinypkg/stray.py"), b"").unwrap();
    std::fs::create_dir(env.pure.join("tinypkg/__pycache__")).unwrap();
    std::fs::write(env.pure.join("tinypkg/__pycache__/x.pyc"), b"").unwrap();
    let err = env.refusal(&["tinypkg"]).replace('\\', "/");
    assert!(
        err.contains("tinypkg/stray.py") && err.contains("no RECORD"),
        "{err}"
    );

    let env = Env::new("lock_resolve_cover_pyc");
    tiny(&env.pure, &[]);
    std::fs::write(env.pure.join("tinypkg.pyc"), b"").unwrap();
    assert!(env.refusal(&["tinypkg"]).contains("tinypkg.pyc"));

    let env = Env::new("lock_resolve_cover_coowner");
    tiny(&env.pure, &[]);
    let co = write_dist(&env.pure, "co", "1", &[("tinypkg/co.py", b"")], &[]);
    std::fs::remove_file(co.join("RECORD")).unwrap();
    assert!(
        env.refusal(&["tinypkg"])
            .replace('\\', "/")
            .contains("tinypkg/co.py")
    );

    let env = Env::new("lock_resolve_cover_location");
    tiny(&env.pure, &[]);
    std::fs::create_dir(env.plat.join("tinypkg")).unwrap();
    std::fs::write(env.plat.join("tinypkg/sub.py"), b"").unwrap();
    let err = env.refusal(&["tinypkg"]).replace('\\', "/");
    assert!(err.contains("plat/tinypkg/sub.py"), "{err}");
}

#[test]
fn a_shared_file_between_locked_packages_needs_the_same_digest() {
    let env = Env::new("lock_resolve_shared_same");
    write_dist(
        &env.pure,
        "ns-a",
        "1",
        &[("ns/__init__.py", b"shared"), ("ns/a.py", b"")],
        &[],
    );
    write_dist(
        &env.pure,
        "ns-b",
        "1",
        &[("ns/__init__.py", b"shared"), ("ns/b.py", b"")],
        &[],
    );
    assert_eq!(env.names(&["ns"]), ["ns-a", "ns-b"]);

    let env = Env::new("lock_resolve_shared_differs");
    write_dist(&env.pure, "ns-a", "1", &[("ns/__init__.py", b"one")], &[]);
    write_dist(&env.plat, "ns-b", "1", &[("ns/__init__.py", b"two")], &[]);
    let err = env.refusal(&["ns"]);
    assert!(
        err.contains("`ns-a` and `ns-b` both install `ns/__init__.py`"),
        "{err}"
    );
}

#[test]
fn missing_site_directories_are_refused_naming_them() {
    let env = Env::new("lock_resolve_missing_site");
    let err = scanned_sites(&env.pure.join("absent"), &env.plat).unwrap_err();
    assert!(
        err.contains("purelib site directory") && err.contains("absent"),
        "{err}"
    );
}

/// A Windows lock owns and walks a top-level `.pyd` import root (#1296):
/// recorded, it locks, where a POSIX lock finds no owner; unrecorded
/// beside a package root, it is refused by name, where a POSIX walk never
/// visits it.
#[test]
fn a_windows_lock_owns_and_walks_a_top_level_pyd() {
    let windows = EmbedPlatform::Windows;
    let pyd = "fast.cp314-win_amd64.pyd";
    let env = Env::new("lock_resolve_windows_pyd");
    write_dist(&env.pure, "fast", "1.0", &[(pyd, b"MZ")], &[]);
    let packages = env.resolve_on(&["fast"], windows).unwrap();
    assert_eq!(packages[0].name, "fast");
    assert_eq!(packages[0].files, 2);
    let err = env.resolve(&["fast"]).unwrap_err();
    assert!(err.contains("no installed distribution owns"), "{err}");

    let env = Env::new("lock_resolve_windows_pyd_stray");
    write_dist(&env.pure, "fast", "1.0", &[("fast/__init__.py", b"")], &[]);
    std::fs::write(env.pure.join(pyd), b"MZ").unwrap();
    let err = env.resolve_on(&["fast"], windows).unwrap_err();
    assert!(err.contains(pyd) && err.contains("no RECORD"), "{err}");
    assert_eq!(env.names(&["fast"]), ["fast"]);
}

/// The Windows RECORD path screen (#1296), as a pure function.
#[test]
fn the_windows_record_path_screen_names_each_defect() {
    for (path, defect) in [
        ("pkg\\..\\x.py", "`\\`"),
        ("C:/x.py", "`:`"),
        ("pkg/x.pyd::$DATA", "`:`"),
        ("pkg/x.pyd.", "strips"),
        ("pkg /x.py", "strips"),
        ("pkg/NUL.py", "device"),
        ("con/x.py", "device"),
        ("pkg/lpt9", "device"),
        ("Com1.tar.gz", "device"),
    ] {
        let found = windows_record_path_defect(path).unwrap_or_else(|| panic!("{path}"));
        assert!(found.contains(defect), "{path}: {found}");
    }
    for path in [
        "pkg/x.py",
        "pkg/./x.py",
        "pkg/../x.py",
        ".hidden/x.py",
        "console.py",
        "pkg/COM10.py",
        "nul_x.py",
    ] {
        assert_eq!(windows_record_path_defect(path), None, "{path}");
    }
}

/// A Windows lock refuses what the screen names, after the `..` refusal
/// and before anything is read, while a POSIX lock is unchanged; `.`
/// components stay admitted on both.
#[test]
fn a_windows_lock_refuses_a_record_path_windows_would_reinterpret() {
    let windows = EmbedPlatform::Windows;
    for (line, why, tag) in [
        (
            "tinypkg\\..\\x.py,sha256=AA,1",
            "contains `\\`",
            "backslash",
        ),
        ("tinypkg/x.pyd::$DATA,sha256=AA,1", "contains `:`", "stream"),
        ("tinypkg/x.pyd.,sha256=AA,1", "strips", "dot"),
        ("tinypkg/NUL.py,sha256=AA,1", "reserves", "device"),
        ("tinypkg/../x.py,sha256=AA,1", "contains `..`", "dotdot"),
    ] {
        let env = Env::new(&format!("lock_resolve_windows_{tag}"));
        let dist = tiny(&env.pure, &[]);
        append_record(&dist, line);
        let err = env.resolve_on(&["tinypkg"], windows).unwrap_err();
        assert!(err.contains(why), "{line}: {err}");
        if tag == "device" {
            // POSIX has no reserved names: it reaches the hash check.
            let err = env.refusal(&["tinypkg"]);
            assert!(err.contains("malformed sha256"), "{err}");
        }
    }
    let env = Env::new("lock_resolve_windows_curdir");
    let dist = tiny(&env.pure, &[]);
    append_record(
        &dist,
        &format!("tinypkg/./sub.py,{},6", record_hash(b"Y = 2\n")),
    );
    assert_eq!(env.resolve_on(&["tinypkg"], windows).unwrap()[0].files, 4);
    assert_eq!(env.names(&["tinypkg"]), ["tinypkg"]);
}

/// #1290: an optional root no distribution owns locks no package, while a
/// required one alongside it still resolves.
#[test]
fn an_absent_optional_root_is_not_a_refusal() {
    let env = Env::new("lock_resolve_optional_absent");
    tiny(&env.pure, &[]);
    assert!(env.resolve_split(&[], &["absentpkg"]).unwrap().is_empty());
    let names: Vec<String> = env
        .resolve_split(&["tinypkg"], &["absentpkg"])
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["tinypkg"]);
    // The same absent root, required, is still refused.
    let err = env.resolve_split(&["absentpkg"], &[]).unwrap_err();
    assert!(
        err.contains("no installed distribution owns import root `absentpkg`"),
        "{err}"
    );
}

/// #1290: an installed optional root is locked with its dependencies,
/// exactly like a required one.
#[test]
fn an_installed_optional_root_locks_its_closure() {
    let env = Env::new("lock_resolve_optional_installed");
    tiny(&env.pure, &["tinydep>=1"]);
    write_dist(
        &env.plat,
        "tinydep",
        "2.0",
        &[("tinydep.py", b"Z = 3\n")],
        &[],
    );
    let packages = env.resolve_split(&[], &["tinypkg"]).unwrap();
    let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["tinydep", "tinypkg"]);
    assert_eq!(packages[1].requires, ["tinydep"]);
}

/// #1290: an unowned optional root with an unrecorded file or link on disk
/// is refused, because CPython would import what the bundle cannot carry.
#[test]
fn an_unowned_optional_root_with_files_on_disk_is_refused() {
    let env = Env::new("lock_resolve_optional_stray");
    std::fs::write(env.pure.join("strayopt.py"), b"X = 1\n").unwrap();
    let err = env.resolve_split(&[], &["strayopt"]).unwrap_err();
    assert!(
        err.contains("strayopt.py")
            && err.contains("optional import root `strayopt`")
            && err.contains("no installed distribution owns"),
        "{err}"
    );

    // A directory holding no file (only an empty `__pycache__`) is a
    // namespace package CPython would import, so it is refused too.
    let env = Env::new("lock_resolve_optional_namespace");
    std::fs::create_dir_all(env.pure.join("nsopt/__pycache__")).unwrap();
    let err = env.resolve_split(&[], &["nsopt"]).unwrap_err();
    assert!(
        err.contains("is a directory for optional import root `nsopt`")
            && err.contains("namespace package"),
        "{err}"
    );

    #[cfg(unix)]
    {
        let env = Env::new("lock_resolve_optional_link");
        std::fs::create_dir_all(env.plat.join("linkopt")).unwrap();
        std::os::unix::fs::symlink(env.plat.join("elsewhere"), env.plat.join("linkopt/m.py"))
            .unwrap();
        let err = env.resolve_split(&[], &["linkopt"]).unwrap_err();
        assert!(
            err.contains("symbolic link") && err.contains("under an import root"),
            "{err}"
        );
    }
}
