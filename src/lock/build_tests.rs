//! The build side of `pycc.lock` (#1242), one test per refusal: every lock
//! here is written by `pycc lock` itself (`run_lock_on`) against a fake
//! interpreter layout and fake site directories, so no python3.14 runs.

use super::*;
use crate::embed::EmbedToolchain;
use crate::embed::fake_layout::{FakeLayout, fake_layout};
use crate::interop_policy::InteropCli;
use crate::lock::fixture::write_dist;
use crate::lock::probe::{parse_lock_probe, tests::probe_lines};
use pycc_scratch::ScratchDir;

const HOST: (&str, &str) = ("aarch64", "macos");
const OTHER_HOST: (&str, &str) = ("riscv64", "linux");

struct Env {
    _dir: ScratchDir,
    layout: FakeLayout,
    pure: PathBuf,
    plat: PathBuf,
    lock_probe: LockProbe,
    entry: PathBuf,
}

impl Env {
    fn new(tag: &str, body: &str) -> Self {
        let dir = ScratchDir::new(tag).unwrap();
        let root = std::fs::canonicalize(&*dir).unwrap();
        let layout_root = root.join("layout");
        std::fs::create_dir_all(&layout_root).unwrap();
        let layout = fake_layout(&layout_root);
        let pure = root.join("pure");
        let plat = root.join("plat");
        std::fs::create_dir_all(&pure).unwrap();
        std::fs::create_dir_all(&plat).unwrap();
        let lock_probe = parse_lock_probe(&probe_lines(&pure, &plat)).unwrap();
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let entry = project.join("m.py");
        std::fs::write(&entry, body).unwrap();
        Self {
            _dir: dir,
            layout,
            pure,
            plat,
            lock_probe,
            entry,
        }
    }

    /// `tinypkg` requiring `tinydep`, both in purelib.
    fn with_tiny(tag: &str, body: &str) -> Self {
        let env = Self::new(tag, body);
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
            &[
                ("tinydep/__init__.py", b"X = 1\n"),
                ("tinydep/data.txt", b"d"),
            ],
            &[],
        );
        env
    }

    fn toolchain(&self) -> EmbedToolchain {
        EmbedToolchain::with_probes("pyfake", self.layout.probe.clone(), self.lock_probe.clone())
    }

    fn write(&self, body: &str) {
        std::fs::write(&self.entry, body).unwrap();
    }

    fn lock(&self) {
        super::super::run_lock_on(
            &self.entry,
            false,
            InteropCli::default(),
            &self.toolchain(),
            HOST,
        )
        .unwrap_or_else(|_| panic!("`pycc lock` must succeed"));
    }

    fn lock_path(&self) -> PathBuf {
        self.entry.with_file_name("pycc.lock")
    }

    fn plan_on(&self, host: (&str, &str)) -> Result<Option<ClosureCheck>, String> {
        let hir = crate::frontend::lock_frontend(&self.entry, InteropCli::default())
            .unwrap_or_else(|_| panic!("the fixture must type-check"));
        plan_closure(&self.entry, &hir, host)
    }

    fn plan(&self) -> Result<Option<ClosureCheck>, String> {
        self.plan_on(HOST)
    }

    fn check(&self) -> ClosureCheck {
        self.plan().unwrap().expect("a section")
    }
}

#[test]
fn a_program_without_a_third_party_root_and_without_a_lock_needs_no_lock_work() {
    let env = Env::new("build_stdlib_no_lock", "import json\nx = 1\n");
    assert!(env.plan().unwrap().is_none());
    env.write("x = 1\n");
    assert!(env.plan().unwrap().is_none());
    // A non-Tier-1 host never computes a triple for it.
    assert!(env.plan_on(OTHER_HOST).unwrap().is_none());
}

#[test]
fn a_third_party_root_without_a_lock_is_refused_naming_pycc_lock() {
    let env = Env::new("build_no_lock", "import tinypkg\n");
    let err = env.plan().unwrap_err();
    assert!(err.contains("imports `tinypkg`"), "{err}");
    assert!(err.contains("which does not exist"), "{err}");
    assert!(
        err.contains(&format!("run `pycc lock {}`", env.entry.display())),
        "{err}"
    );
}

#[test]
fn a_non_tier_1_host_skips_a_stdlib_only_lock_and_refuses_a_third_party_root() {
    let env = Env::with_tiny("build_non_tier1", "import json\n");
    env.lock();
    assert!(env.lock_path().is_file());
    assert!(env.plan_on(OTHER_HOST).unwrap().is_none());
    env.write("import tinypkg\n");
    let err = env.plan_on(OTHER_HOST).unwrap_err();
    assert!(
        err.contains("an embedded build cannot use `pycc.lock` here"),
        "{err}"
    );
    assert!(err.contains("not one"), "{err}");
}

#[test]
fn a_malformed_lock_is_refused_even_for_a_stdlib_only_program() {
    let env = Env::new("build_malformed", "import json\n");
    std::fs::write(env.lock_path(), "not a lock\n").unwrap();
    let err = env.plan().unwrap_err();
    assert!(
        err.contains(&format!("cannot use `{}`", env.lock_path().display())),
        "{err}"
    );
    // The host is not consulted first: a non-Tier-1 host refuses it too.
    assert!(env.plan_on(OTHER_HOST).unwrap_err().contains("cannot use"));
    env.write("import tinypkg\n");
    assert!(env.plan().unwrap_err().contains("cannot use"));
}

#[test]
fn a_lock_without_this_entry_s_section() {
    let env = Env::with_tiny("build_no_section", "import tinypkg\n");
    let other = env.entry.with_file_name("other.py");
    std::fs::write(&other, "import tinypkg\n").unwrap();
    super::super::run_lock_on(&other, false, InteropCli::default(), &env.toolchain(), HOST)
        .unwrap_or_else(|_| panic!("lock"));
    let err = env.plan().unwrap_err();
    assert!(
        err.contains("no `pycc.lock` section for `m.py` on `aarch64-apple-darwin`"),
        "{err}"
    );
    assert!(err.contains("run `pycc lock"), "{err}");
    // A stdlib-only program with no section needs no lock work.
    env.write("import json\n");
    assert!(env.plan().unwrap().is_none());
}

#[test]
fn a_section_whose_roots_differ_from_the_program_is_stale() {
    let env = Env::with_tiny("build_roots_differ", "import tinypkg\n");
    env.lock();
    assert_eq!(env.check().section.roots, ["tinypkg"]);
    env.write("import tinypkg\nimport tinydep\n");
    let err = env.plan().unwrap_err();
    assert!(
        err.contains(
            "its section's `roots` lists `tinypkg` but the program requires `tinydep`, `tinypkg`"
        ),
        "{err}"
    );
    assert!(err.contains("does not match this build"), "{err}");
    // A stdlib-only program against a section that still lists roots.
    env.write("import json\n");
    let err = env.plan().unwrap_err();
    assert!(
        err.contains("lists `tinypkg` but the program requires no roots"),
        "{err}"
    );
}

/// #1290: a program whose only third-party import is optional still needs
/// its lock, and the refusal names the optional root.
#[test]
fn an_optional_only_program_without_a_lock_is_refused_naming_its_root() {
    let env = Env::new(
        "build_optional_no_lock",
        "try:\n    import absentpkg\nexcept ImportError:\n    pass\n",
    );
    let err = env.plan().unwrap_err();
    assert!(
        err.contains("the program imports `absentpkg` from outside the standard library"),
        "{err}"
    );
    assert!(err.contains("which does not exist"), "{err}");
}

/// #1290: an absent optional root locks as `optional-roots` with no
/// package, the build plans no files for it, and a section whose
/// `optional-roots` differs from the program's is stale, naming the field.
#[test]
fn an_absent_optional_root_locks_and_builds_with_no_files() {
    let env = Env::new(
        "build_optional_absent",
        "try:\n    import absentpkg\nexcept ImportError:\n    pass\n",
    );
    env.lock();
    let text = std::fs::read_to_string(env.lock_path()).unwrap();
    assert!(
        text.contains("roots = []\noptional-roots = [\"absentpkg\"]\n"),
        "{text}"
    );
    let check = env.check();
    assert!(check.section.package.is_empty());
    let closure = payload(&check, &env.lock_probe, EmbedPlatform::Linux).unwrap();
    assert!(closure.files.is_empty());

    // The same import made unconditional: `roots` differs first.
    env.write("import absentpkg\n");
    let err = env.plan().unwrap_err();
    assert!(
        err.contains("`roots` lists no roots but the program requires `absentpkg`"),
        "{err}"
    );
    // Another optional root: `optional-roots` differs.
    env.write("try:\n    import otherpkg\nexcept ImportError:\n    pass\n");
    let err = env.plan().unwrap_err();
    assert!(
        err.contains(
            "its section's `optional-roots` lists `absentpkg` but the program imports \
             `otherpkg` from outside the standard library only under a handler that catches a failed import"
        ),
        "{err}"
    );
    assert!(err.contains("does not match this build"), "{err}");
}

/// #1290: an installed optional root is locked with its closure and
/// bundled, and one required elsewhere in the program is required.
#[test]
fn an_installed_optional_root_is_locked_with_its_closure() {
    let env = Env::with_tiny(
        "build_optional_installed",
        "try:\n    import tinypkg\nexcept ImportError:\n    pass\n",
    );
    env.lock();
    let check = env.check();
    assert!(check.section.roots.is_empty());
    assert_eq!(check.section.optional_roots, ["tinypkg"]);
    let names: Vec<&str> = check
        .section
        .package
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, ["tinydep", "tinypkg"]);

    env.write("import tinypkg\ntry:\n    import tinypkg\nexcept ImportError:\n    pass\n");
    env.lock();
    let check = env.check();
    assert_eq!(check.section.roots, ["tinypkg"]);
    assert!(check.section.optional_roots.is_empty());
}

/// #1383: a nested from-import is a root like a nested `import`: optional
/// under a handler that catches a failed import, required under an `if`.
#[test]
fn a_nested_from_import_is_an_optional_or_a_required_root() {
    let env = Env::with_tiny(
        "build_nested_from",
        "try:\n    from tinypkg import X\nexcept ImportError:\n    pass\n",
    );
    env.lock();
    let check = env.check();
    assert!(check.section.roots.is_empty());
    assert_eq!(check.section.optional_roots, ["tinypkg"]);

    env.write("c = True\nif c:\n    from tinypkg import X\n");
    env.lock();
    let check = env.check();
    assert_eq!(check.section.roots, ["tinypkg"]);
    assert!(check.section.optional_roots.is_empty());
}

/// Native libraries no longer stop the plan: the embedded build re-derives
/// them after the probe and compares them with the section (#1243).
#[test]
fn a_section_with_native_libraries_is_planned() {
    let env = Env::with_tiny("build_native", "import tinypkg\n");
    env.lock();
    let mut lock = schema::parse(&std::fs::read_to_string(env.lock_path()).unwrap()).unwrap();
    lock.target[0].native.push(schema::LockedNative {
        name: "libx.dylib".to_string(),
        sha256: "00".repeat(32),
        required_by: vec!["tinypkg".to_string()],
    });
    std::fs::write(env.lock_path(), schema::render(&lock)).unwrap();
    let check = env.plan().expect("planned").expect("a section");
    assert_eq!(check.section.native.len(), 1);
}

#[test]
fn the_interpreter_fields_are_compared_one_at_a_time() {
    let env = Env::with_tiny("build_interpreter", "import tinypkg\n");
    env.lock();
    let check = env.check();
    let probe = &env.layout.probe;
    assert_eq!(verify_interpreter(&check, probe, &env.lock_probe), Ok(()));
    let mut older = probe.clone();
    older.version = (3, 14, 6);
    let err = verify_interpreter(&check, &older, &env.lock_probe).unwrap_err();
    assert!(
        err.contains("`python` is `3.14.7` in the lock but `3.14.6` in the environment"),
        "{err}"
    );
    assert!(err.contains("run `pycc lock"), "{err}");
    let mut tagged = env.lock_probe.clone();
    tagged.cache_tag = "cpython-315".to_string();
    let err = verify_interpreter(&check, probe, &tagged).unwrap_err();
    assert!(err.contains("`cache-tag`"), "{err}");
    let mut moved = env.lock_probe.clone();
    moved.platform = "linux-x86_64".to_string();
    let err = verify_interpreter(&check, probe, &moved).unwrap_err();
    assert!(err.contains("`platform`"), "{err}");
}

#[test]
fn the_payload_plans_every_locked_file_once() {
    let env = Env::with_tiny("build_payload", "import tinypkg\n");
    env.lock();
    let check = env.check();
    let closure = payload(&check, &env.lock_probe, EmbedPlatform::Linux).unwrap();
    let rels: Vec<&str> = closure.files.iter().map(|file| file.rel.as_str()).collect();
    assert!(rels.contains(&"tinypkg/__init__.py"), "{rels:?}");
    assert!(rels.contains(&"tinydep/data.txt"), "{rels:?}");
    assert!(
        rels.iter()
            .any(|rel| rel.starts_with("tinydep-2.0.dist-info/")),
        "{rels:?}"
    );
    let data = closure
        .files
        .iter()
        .find(|file| file.rel == "tinydep/data.txt")
        .unwrap();
    assert_eq!(data.source, env.pure.join("tinydep/data.txt"));
    assert_eq!(data.digest, crate::embed::sha256::sha256_hex(b"d"));
    assert_eq!(closure.owner_of("tinydep/data.txt"), "tinydep");
    assert_eq!(closure.owner_of("absent"), "");
    let canonical = |path: &PathBuf| std::fs::canonicalize(path).unwrap();
    assert_eq!(closure.sites, [canonical(&env.pure), canonical(&env.plat)]);
    assert_eq!(closure.libpython_sha256, check.section.libpython_sha256);
    let copied: BTreeMap<String, String> = closure
        .files
        .iter()
        .map(|file| (file.rel.clone(), file.digest.clone()))
        .collect();
    assert_eq!(closure.verify_copied(&copied), Ok(()));
}

#[test]
fn a_stdlib_only_section_plans_no_files() {
    let env = Env::new("build_stdlib_section", "import json\n");
    env.lock();
    let check = env.check();
    assert!(check.section.roots.is_empty());
    let closure = payload(&check, &env.lock_probe, EmbedPlatform::Linux).unwrap();
    assert!(closure.files.is_empty());
    assert_eq!(closure.verify_copied(&BTreeMap::new()), Ok(()));
}

#[test]
fn a_copied_payload_that_does_not_add_up_to_the_lock_is_stale() {
    let env = Env::with_tiny("build_verify_copied", "import tinypkg\n");
    env.lock();
    let mut check = env.check();
    check.section.package[0].files += 1;
    let closure = payload(&check, &env.lock_probe, EmbedPlatform::Linux).unwrap();
    let copied: BTreeMap<String, String> = closure
        .files
        .iter()
        .map(|file| (file.rel.clone(), file.digest.clone()))
        .collect();
    let err = closure.verify_copied(&copied).unwrap_err();
    assert!(
        err.contains("the copied payload of distribution `tinydep` differs: `files`"),
        "{err}"
    );
    let mut check = env.check();
    check.section.package[1].tree_sha256 = "00".repeat(32);
    let closure = payload(&check, &env.lock_probe, EmbedPlatform::Linux).unwrap();
    let err = closure.verify_copied(&copied).unwrap_err();
    assert!(err.contains("`tinypkg` differs: `tree-sha256`"), "{err}");
}

#[test]
fn a_locked_distribution_must_be_installed_exactly_once_at_its_version() {
    let env = Env::with_tiny("build_installed", "import tinypkg\n");
    env.lock();

    let mut check = env.check();
    check.section.package[0].site = "elsewhere".to_string();
    let err = payload(&check, &env.lock_probe, EmbedPlatform::Linux).unwrap_err();
    assert!(
        err.contains("locked in the elsewhere site directory"),
        "{err}"
    );

    let mut check = env.check();
    check.section.package[0].version = "9.9".to_string();
    let err = payload(&check, &env.lock_probe, EmbedPlatform::Linux).unwrap_err();
    assert!(
        err.contains("`tinydep` is locked at version `9.9` but `2.0` is installed"),
        "{err}"
    );

    write_dist(&env.pure, "TinyDep", "3.0", &[], &[]);
    let err = payload(&env.check(), &env.lock_probe, EmbedPlatform::Linux).unwrap_err();
    assert!(
        err.contains("`tinydep` is installed more than once"),
        "{err}"
    );
    std::fs::remove_dir_all(env.pure.join("TinyDep-3.0.dist-info")).unwrap();

    std::fs::remove_dir_all(env.pure.join("tinydep-2.0.dist-info")).unwrap();
    let err = payload(&env.check(), &env.lock_probe, EmbedPlatform::Linux).unwrap_err();
    assert!(err.contains("`tinydep` is not installed in"), "{err}");
}

#[test]
fn an_unreadable_metadata_or_record_is_refused() {
    let env = Env::with_tiny("build_unreadable", "import tinypkg\n");
    env.lock();
    let dist_info = env.pure.join("tinydep-2.0.dist-info");
    let record = std::fs::read_to_string(dist_info.join("RECORD")).unwrap();
    std::fs::remove_file(dist_info.join("RECORD")).unwrap();
    let err = payload(&env.check(), &env.lock_probe, EmbedPlatform::Linux).unwrap_err();
    assert!(err.contains("RECORD"), "{err}");
    std::fs::write(dist_info.join("RECORD"), record).unwrap();

    std::fs::write(dist_info.join("METADATA"), "Metadata-Version: 2.1\n").unwrap();
    let err = payload(&env.check(), &env.lock_probe, EmbedPlatform::Linux).unwrap_err();
    assert!(err.contains("cannot read `"), "{err}");
    assert!(err.contains("METADATA"), "{err}");
    std::fs::remove_file(dist_info.join("METADATA")).unwrap();
    let err = payload(&env.check(), &env.lock_probe, EmbedPlatform::Linux).unwrap_err();
    assert!(err.contains("METADATA"), "{err}");
}

#[test]
fn the_shared_record_classifier_refuses_a_missing_payload_file() {
    let env = Env::with_tiny("build_classifier", "import tinypkg\n");
    env.lock();
    std::fs::remove_file(env.pure.join("tinydep/data.txt")).unwrap();
    let err = payload(&env.check(), &env.lock_probe, EmbedPlatform::Linux).unwrap_err();
    assert!(err.contains("tinydep/data.txt"), "{err}");
}

#[test]
fn a_shared_path_is_planned_once_and_counted_for_every_claimant() {
    let env = Env::new("build_shared", "import ns\n");
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
    env.lock();
    let check = env.check();
    let closure = payload(&check, &env.lock_probe, EmbedPlatform::Linux).unwrap();
    let shared: Vec<&ClosureFile> = closure
        .files
        .iter()
        .filter(|file| file.rel == "ns/__init__.py")
        .collect();
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].package, "ns-a");
    let copied: BTreeMap<String, String> = closure
        .files
        .iter()
        .map(|file| (file.rel.clone(), file.digest.clone()))
        .collect();
    assert_eq!(closure.verify_copied(&copied), Ok(()));

    // The second claim names a different RECORD digest.
    let record = env.pure.join("ns-b-1.dist-info").join("RECORD");
    let text = std::fs::read_to_string(&record).unwrap();
    let edited = text.replace(
        &crate::lock::fixture::record_hash(b"shared"),
        &crate::lock::fixture::record_hash(b"other"),
    );
    std::fs::write(&record, edited).unwrap();
    let err = payload(&check, &env.lock_probe, EmbedPlatform::Linux).unwrap_err();
    assert!(
        err.contains(
            "distributions `ns-a` and `ns-b` both install `ns/__init__.py` with different contents"
        ),
        "{err}"
    );
}

#[test]
fn a_shared_path_claimed_from_two_site_directories_is_stale() {
    let env = Env::new("build_shared_sites", "import ns\n");
    write_dist(&env.pure, "ns-a", "1", &[("ns/__init__.py", b"s")], &[]);
    write_dist(&env.pure, "ns-b", "1", &[("ns/__init__.py", b"s")], &[]);
    env.lock();
    // `ns-b` moves to platlib after the lock, carrying its own copy.
    std::fs::remove_dir_all(env.pure.join("ns-b-1.dist-info")).unwrap();
    write_dist(&env.plat, "ns-b", "1", &[("ns/__init__.py", b"s")], &[]);
    let mut check = env.check();
    check.section.package[1].site = "platlib".to_string();
    let err = payload(&check, &env.lock_probe, EmbedPlatform::Linux).unwrap_err();
    assert!(err.contains("both install `ns/__init__.py`"), "{err}");
}
