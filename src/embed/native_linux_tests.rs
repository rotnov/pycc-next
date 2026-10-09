//! The Linux dependency scan over synthetic ELF images, on any host: a
//! fake interpreter prefix, one system directory, and libraries outside
//! both.

use super::super::elf::fixture::{AARCH64, ElfSpec, elf_bytes};
use super::super::fake_layout::{FakeLayout, fake_layout};
use super::*;
use crate::lock::build::ClosureFile;
use pycc_scratch::ScratchDir;

struct Fixture {
    _dir: ScratchDir,
    root: PathBuf,
    layout: FakeLayout,
    env: LinuxEnv,
    files: Vec<ClosureFile>,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = ScratchDir::new(tag).expect("scratch");
        let root = std::fs::canonicalize(&*dir).expect("canonical");
        let layout = fake_layout(&root);
        let env = LinuxEnv {
            system_dirs: vec![root.join("sys")],
            ldconfig_programs: vec![root.join("no-ldconfig")],
        };
        let fixture = Self {
            _dir: dir,
            root,
            layout,
            env,
            files: Vec::new(),
        };
        fixture.write("sys/libc.so.6", &ElfSpec::library("libc.so.6", &[]));
        fixture
    }

    /// Writes `spec` at `rel` under the scratch root and returns its path.
    fn write(&self, rel: &str, spec: &ElfSpec) -> PathBuf {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(&path, elf_bytes(spec)).expect("write");
        path
    }

    /// Adds a closure image `rel` of distribution `package`, installed
    /// under `site/`.
    fn image(&mut self, rel: &str, package: &str, spec: &ElfSpec) {
        let source = self.write(&format!("site/{rel}"), spec);
        self.files.push(ClosureFile {
            rel: rel.to_string(),
            source,
            digest: String::new(),
            package: package.to_string(),
        });
    }

    fn outside(&self) -> String {
        self.root.join("outside").display().to_string()
    }

    fn plan(&self, interpreter: bool) -> Result<NativePlan, String> {
        self.plan_linked(interpreter, LibpythonLink::Shared)
    }

    fn plan_linked(&self, interpreter: bool, link: LibpythonLink) -> Result<NativePlan, String> {
        let closure = LockedClosure::of_files(self.files.clone());
        plan(
            &self.layout.probe,
            Some(&closure),
            &self.env,
            interpreter,
            link,
        )
    }
}

fn names(plan: &NativePlan) -> Vec<&str> {
    let natives = plan.natives.iter();
    natives.map(|native| native.locked.name.as_str()).collect()
}

fn vendor_names(plan: &NativePlan) -> Vec<&str> {
    plan.linux_vendor
        .iter()
        .map(|(name, _)| name.as_str())
        .collect()
}

#[test]
fn the_host_environment_names_the_default_and_multiarch_directories() {
    let env = LinuxEnv::host();
    for dir in [
        "/lib",
        "/usr/lib64",
        "/usr/lib/x86_64-linux-gnu",
        "/lib/aarch64-linux-gnu",
    ] {
        assert!(env.system_dirs.contains(&PathBuf::from(dir)), "{dir}");
    }
    assert_eq!(env.system_dirs.len(), 8);
    let programs = [PathBuf::from("/sbin/ldconfig"), PathBuf::from("ldconfig")];
    assert_eq!(env.ldconfig_programs, programs);
}

/// A chain of natives reached from two distributions is recorded once per
/// library with both owners; a prefix library is vendored but is not a
/// native; a payload sibling reached through `$ORIGIN` and a system
/// library are kept.
#[test]
fn closure_images_reach_natives_transitively_for_every_owner() {
    let mut fx = Fixture::new("native_linux_chain");
    let outside = fx.outside();
    fx.write(
        "outside/libnat1.so.1",
        &ElfSpec::library("libnat1.so.1", &["libnat2.so.2", "libc.so.6"]).runpath("$ORIGIN"),
    );
    fx.write(
        "outside/libnat2.so.2",
        &ElfSpec::library("libnat2.so.2", &[]),
    );
    let prefix_lib = fx.layout.prefix.join("lib");
    fx.write(
        "prefix/lib/libffi.so.8",
        &ElfSpec::library("libffi.so.8", &["libc.so.6"]),
    );
    fx.image(
        "pa/libsib.so",
        "pa",
        &ElfSpec::library("libsib.so", &["libc.so.6"]),
    );
    let search = format!("$ORIGIN:{outside}:{}", prefix_lib.display());
    let needs = [
        "libsib.so",
        "libnat1.so.1",
        "libffi.so.8",
        "libmissing.so.9",
    ];
    fx.image("pa/_a.so", "pa", &ElfSpec::module(&needs).runpath(&search));
    fx.image(
        "pb/_b.so",
        "pb",
        &ElfSpec::module(&["libnat1.so.1"]).rpath(&outside),
    );
    // Neither a Python file nor an empty file is an image.
    std::fs::write(fx.root.join("site/pa/__init__.py"), "X = 1\n").unwrap();
    let source = fx.root.join("site/pa/__init__.py");
    fx.files.push(ClosureFile {
        rel: "pa/__init__.py".to_string(),
        source,
        digest: String::new(),
        package: "pa".to_string(),
    });
    let plan = fx.plan(false).expect("planned");
    assert_eq!(names(&plan), ["libnat1.so.1", "libnat2.so.2"]);
    for native in &plan.natives {
        assert_eq!(native.locked.required_by, ["pa", "pb"]);
        let digest = crate::embed::sha256::sha256_file(&native.source).unwrap();
        assert_eq!(native.locked.sha256, digest);
    }
    assert_eq!(
        vendor_names(&plan),
        ["libffi.so.8", "libnat1.so.1", "libnat2.so.2"]
    );
    let (_, ffi) = &plan.linux_vendor[0];
    assert_eq!(ffi, &prefix_lib.join("libffi.so.8"));
    assert_eq!(plan.locked().len(), 2);
    let nat2 = fx.root.join("outside/libnat2.so.2");
    assert_eq!(
        plan.native_at(&nat2).map(|n| n.name.as_str()),
        Some("libnat2.so.2")
    );
    assert_eq!(plan.native_at(&ffi.clone()), None);
}

/// With `interpreter`, libpython and every kept `lib-dynload` extension
/// are scanned too: a prefix library they need is vendored, and one
/// outside the prefix is refused naming the extension.
#[test]
fn the_interpreter_images_vendor_prefix_libraries_and_refuse_outside_ones() {
    let fx = Fixture::new("native_linux_interpreter");
    let library = fx.layout.library();
    std::fs::write(
        &library,
        elf_bytes(&ElfSpec::library(
            "libpython3.14.so.1.0",
            &["libc.so.6", "libutil.so.1"],
        )),
    )
    .unwrap();
    let prefix_lib = fx.layout.prefix.join("lib");
    let rpath = prefix_lib.display().to_string();
    fx.write(
        "prefix/lib/libutil.so.1",
        &ElfSpec::library("libutil.so.1", &["libc.so.6"]),
    );
    fx.write("prefix/lib/libz.so.1", &ElfSpec::library("libz.so.1", &[]));
    let dynload = fx.layout.dynload();
    let needs = ["libz.so.1", "libutil.so.1", "libpython3.14.so.1.0"];
    let zlib = ElfSpec::module(&needs).runpath(&rpath);
    std::fs::write(
        dynload.join("zlib.cpython-314-x86_64-linux-gnu.so"),
        elf_bytes(&zlib),
    )
    .unwrap();
    // A second extension needing `libz` copies it once.
    let binascii = ElfSpec::module(&["libz.so.1"]).runpath(&rpath);
    std::fs::write(
        dynload.join("binascii.cpython-314-x86_64-linux-gnu.so"),
        elf_bytes(&binascii),
    )
    .unwrap();
    // A directory and a dangling link in `lib-dynload` are not images.
    std::fs::create_dir(dynload.join("subdir")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(fx.root.join("nowhere"), dynload.join("dangling.so")).unwrap();
    let plan = fx.plan(true).expect("planned");
    assert_eq!(vendor_names(&plan), ["libutil.so.1", "libz.so.1"]);
    assert!(plan.natives.is_empty());
    // Without `interpreter`, none of that is scanned.
    assert!(fx.plan(false).expect("planned").linux_vendor.is_empty());

    let outside = fx.outside();
    fx.write("outside/libbad.so.1", &ElfSpec::library("libbad.so.1", &[]));
    let bad = ElfSpec::module(&["libbad.so.1"]).runpath(&outside);
    std::fs::write(
        dynload.join("_bad.cpython-314-x86_64-linux-gnu.so"),
        elf_bytes(&bad),
    )
    .unwrap();
    let err = fx.plan(true).expect_err("refused");
    assert!(err.contains("embed interpreter"), "{err}");
    assert!(err.contains("is not relocatable"), "{err}");
    assert!(
        err.contains("`python3.14/lib-dynload/_bad.cpython-314-x86_64-linux-gnu.so`"),
        "{err}"
    );
    assert!(err.contains("`libbad.so.1`"), "{err}");
}

/// The interpreter's own library reached under a name the bundle does not
/// provide would load a second copy, so it is refused; under the bundled
/// name it is kept.
#[test]
fn libpython_is_kept_by_its_bundled_name_and_refused_by_any_other() {
    let mut fx = Fixture::new("native_linux_libpython");
    let prefix_lib = fx.layout.prefix.join("lib").display().to_string();
    std::fs::write(
        fx.layout.library(),
        elf_bytes(&ElfSpec::library("libpython3.14.dylib", &[])),
    )
    .unwrap();
    fx.write(
        "sys/libpython3.14.so.1.0",
        &ElfSpec::library("libpython3.14.so.1.0", &[]),
    );
    fx.image("p/_ok.so", "p", &ElfSpec::module(&["libpython3.14.so.1.0"]));
    assert!(fx.plan(false).expect("kept").linux_vendor.is_empty());
    fx.image(
        "p/_bad.so",
        "p",
        &ElfSpec::module(&["libpython3.14.dylib"]).runpath(&prefix_lib),
    );
    let err = fx.plan(false).expect_err("second libpython");
    assert!(
        err.contains("`closure/p/_bad.so` (distribution `p`)"),
        "{err}"
    );
    assert!(err.contains("the interpreter's own library"), "{err}");
    assert!(err.contains("`libpython3.14.so.1.0`"), "{err}");
}

/// A static build (D-251) scans no libpython, and refuses any image that
/// needs a shared one: by name before resolving it, since a static-only
/// host may have none to resolve to.
#[test]
fn a_static_build_skips_libpython_and_refuses_an_image_that_needs_one() {
    let fx = Fixture::new("native_linux_static");
    let static_link = LibpythonLink::Static;
    let dynload = fx.layout.dynload();
    let plain = ElfSpec::module(&["libc.so.6"]);
    let plain_name = "_plain.cpython-314-x86_64-linux-gnu.so";
    std::fs::write(dynload.join(plain_name), elf_bytes(&plain)).unwrap();
    let plan = fx.plan_linked(true, static_link).expect("planned");
    assert!(plan.linux_vendor.is_empty() && plan.natives.is_empty());
    let needs = ElfSpec::module(&["libc.so.6", "libpython3.14.so.1.0"]);
    let needs_name = "_json.cpython-314-x86_64-linux-gnu.so";
    std::fs::write(dynload.join(needs_name), elf_bytes(&needs)).unwrap();
    let err = fx.plan_linked(true, static_link).expect_err("refused");
    let image = format!("`python3.14/lib-dynload/{needs_name}`");
    assert!(err.contains(&image), "{err}");
    assert!(
        err.contains("depends on `libpython3.14.so.1.0`, a shared libpython"),
        "{err}"
    );
    assert!(err.contains("links libpython statically"), "{err}");
}

/// Under a name that does not look like libpython, the source library is
/// still caught once the name resolves to it.
#[cfg(unix)]
#[test]
fn a_static_build_refuses_the_source_library_under_another_name() {
    let mut fx = Fixture::new("native_linux_static_alias");
    let prefix_lib = fx.layout.prefix.join("lib");
    std::fs::write(
        fx.layout.library(),
        elf_bytes(&ElfSpec::library("libpython3.14.dylib", &[])),
    )
    .unwrap();
    std::os::unix::fs::symlink(fx.layout.library(), prefix_lib.join("libalias.so")).unwrap();
    let runpath = prefix_lib.display().to_string();
    fx.image(
        "p/_alias.so",
        "p",
        &ElfSpec::module(&["libalias.so"]).runpath(&runpath),
    );
    let err = fx
        .plan_linked(false, LibpythonLink::Static)
        .expect_err("second libpython");
    assert!(
        err.starts_with("`closure/p/_alias.so` (distribution `p`) depends on `libalias.so`"),
        "{err}"
    );
    assert!(err.contains("a shared libpython"), "{err}");
}

#[test]
fn a_malformed_image_is_refused_by_path() {
    let mut fx = Fixture::new("native_linux_malformed");
    let bytes = elf_bytes(&ElfSpec::module(&["libx.so"]));
    let path = fx.root.join("site/p/_cut.so");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes[..bytes.len() - 20]).unwrap();
    fx.files.push(ClosureFile {
        rel: "p/_cut.so".to_string(),
        source: path.clone(),
        digest: String::new(),
        package: "p".to_string(),
    });
    let err = fx.plan(false).expect_err("malformed");
    assert!(
        err.contains(&format!("cannot scan `{}`", path.display())),
        "{err}"
    );
}

#[test]
fn an_unreadable_closure_source_is_refused() {
    let mut fx = Fixture::new("native_linux_unreadable");
    fx.files.push(ClosureFile {
        rel: "p/_gone.so".to_string(),
        source: fx.root.join("site/p/_gone.so"),
        digest: String::new(),
        package: "p".to_string(),
    });
    let err = fx.plan(false).expect_err("missing");
    assert!(err.contains("could not read"), "{err}");
}

/// The loader matches a preloaded library by its `DT_SONAME`, so a copied
/// library whose soname is not the name it is needed by is refused, from
/// a closure image and from a native alike.
#[test]
fn a_copied_library_must_carry_the_needed_name_as_its_soname() {
    let mut fx = Fixture::new("native_linux_soname");
    let outside = fx.outside();
    fx.write("outside/libold.so.1", &ElfSpec::library("libnew.so.2", &[]));
    fx.image(
        "p/_a.so",
        "p",
        &ElfSpec::module(&["libold.so.1"]).runpath(&outside),
    );
    let err = fx.plan(false).expect_err("soname");
    assert!(err.contains("needs `libold.so.1`"), "{err}");
    assert!(err.contains("whose `DT_SONAME` is `libnew.so.2`"), "{err}");

    let mut fx = Fixture::new("native_linux_no_soname");
    let outside = fx.outside();
    fx.write(
        "outside/libnat.so.1",
        &ElfSpec::library("libnat.so.1", &["libplain.so"]).runpath("$ORIGIN"),
    );
    fx.write("outside/libplain.so", &ElfSpec::module(&[]));
    fx.image(
        "p/_a.so",
        "p",
        &ElfSpec::module(&["libnat.so.1"]).runpath(&outside),
    );
    let err = fx.plan(false).expect_err("no soname");
    assert!(
        err.contains("the native library `lib/libnat.so.1` (for distribution `p`)"),
        "{err}"
    );
    assert!(err.contains("whose `DT_SONAME` is missing"), "{err}");
}

/// A program in the closure keeps its own dynamic loader search, so a
/// library it needs cannot be satisfied by the bundle's preload.
#[test]
fn a_program_image_that_needs_a_copied_library_is_refused() {
    let mut fx = Fixture::new("native_linux_program");
    let outside = fx.outside();
    fx.write("outside/libnat.so.1", &ElfSpec::library("libnat.so.1", &[]));
    let mut tool = ElfSpec::module(&["libnat.so.1", "libc.so.6"]).runpath(&outside);
    tool.program = true;
    fx.image("p/bin/tool", "p", &tool);
    let err = fx.plan(false).expect_err("program");
    assert!(
        err.contains("`closure/p/bin/tool` (distribution `p`) is a program"),
        "{err}"
    );
    assert!(err.contains("`libnat.so.1`"), "{err}");
}

/// A copied library whose name another image resolves to a kept system
/// library would answer that dependency too once preloaded, so the two
/// cannot coexist; the same name kept twice, or kept and copied from the
/// same path, is fine.
#[test]
fn a_copied_library_may_not_shadow_a_kept_one() {
    let mut fx = Fixture::new("native_linux_shadow");
    let outside = fx.outside();
    fx.write("sys/libssl.so.3", &ElfSpec::library("libssl.so.3", &[]));
    fx.write("outside/libssl.so.3", &ElfSpec::library("libssl.so.3", &[]));
    fx.image(
        "p/_a.so",
        "p",
        &ElfSpec::module(&["libssl.so.3", "libc.so.6"]),
    );
    fx.image("p/_c.so", "p", &ElfSpec::module(&["libssl.so.3"]));
    assert!(fx.plan(false).expect("kept twice").linux_vendor.is_empty());
    fx.image(
        "q/_b.so",
        "q",
        &ElfSpec::module(&["libssl.so.3"]).runpath(&outside),
    );
    let err = fx.plan(false).expect_err("shadowed");
    assert!(err.contains("copies `libssl.so.3`"), "{err}");
    assert!(
        err.contains("`closure/p/_a.so` (distribution `p`)"),
        "{err}"
    );
}

/// Two different libraries needed by the same name cannot both be copied.
#[test]
fn two_natives_needed_by_one_name_are_refused() {
    let mut fx = Fixture::new("native_linux_collision");
    let one = fx.root.join("one").display().to_string();
    let two = fx.root.join("two").display().to_string();
    fx.write("one/libx.so.1", &ElfSpec::library("libx.so.1", &[]));
    fx.write(
        "two/libx.so.1",
        &ElfSpec::library("libx.so.1", &["libc.so.6"]),
    );
    fx.image(
        "p/_a.so",
        "p",
        &ElfSpec::module(&["libx.so.1"]).runpath(&one),
    );
    fx.image(
        "q/_b.so",
        "q",
        &ElfSpec::module(&["libx.so.1"]).runpath(&two),
    );
    let err = fx.plan(false).expect_err("collision");
    assert!(err.contains("both need the name `libx.so.1`"), "{err}");
}

/// `DT_NEEDED` resolution: an absolute name is taken as is, a relative
/// path is left to the loader, a library for another machine is passed
/// over for the next directory, `DT_RUNPATH` wins over `DT_RPATH`, and the
/// loader cache and the default directories come last.
#[test]
fn dependencies_resolve_in_the_loader_order() {
    let mut fx = Fixture::new("native_linux_resolve");
    let libc = fx.root.join("sys/libc.so.6").display().to_string();
    let mut foreign = ElfSpec::library("libm1.so", &[]);
    foreign.machine = AARCH64;
    fx.write("first/libm1.so", &foreign);
    fx.write("second/libm1.so", &ElfSpec::library("libm1.so", &[]));
    let search = format!(
        "{}:{}",
        fx.root.join("first").display(),
        fx.root.join("second").display()
    );
    let needs = [libc.as_str(), "sub/librel.so", "libm1.so", "libnowhere.so"];
    // `DT_RPATH` names a directory that does not have `libm1.so`.
    let spec = ElfSpec::module(&needs)
        .rpath("/pycc-test-nowhere")
        .runpath(&search);
    fx.image("p/_a.so", "p", &spec);
    let plan = fx.plan(false).expect("planned");
    assert_eq!(names(&plan), ["libm1.so"]);
    let (_, m1) = &plan.linux_vendor[0];
    assert_eq!(m1, &fx.root.join("second/libm1.so"));

    // A library needed by its absolute path would still be opened from
    // that path on the target machine.
    let abs = fx.root.join("abs/libabs.so").display().to_string();
    fx.write("abs/libabs.so", &ElfSpec::library(&abs, &[]));
    fx.image("p/_b.so", "p", &ElfSpec::module(&[abs.as_str()]));
    let err = fx.plan(false).expect_err("absolute");
    assert!(err.contains(&format!("needs `{abs}` by its path")), "{err}");
}

/// The first `ldconfig` that succeeds supplies the loader cache; one that
/// cannot start or fails is passed over.
#[cfg(unix)]
#[test]
fn the_loader_cache_comes_from_the_first_ldconfig_that_succeeds() {
    use std::os::unix::fs::PermissionsExt;
    let mut fx = Fixture::new("native_linux_ldconfig");
    fx.write(
        "cached/libcache.so.1",
        &ElfSpec::library("libcache.so.1", &[]),
    );
    let failing = fx.root.join("ldconfig-fails");
    std::fs::write(&failing, "#!/bin/sh\nexit 1\n").unwrap();
    let working = fx.root.join("ldconfig-works");
    let listing = format!(
        "#!/bin/sh\necho '1 libs found in cache'\necho '\tlibcache.so.1 (libc6,x86-64) => {}'\n",
        fx.root.join("cached/libcache.so.1").display()
    );
    std::fs::write(&working, listing).unwrap();
    for script in [&failing, &working] {
        std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    fx.env.ldconfig_programs = vec![fx.root.join("absent"), failing, working];
    fx.image("p/_a.so", "p", &ElfSpec::module(&["libcache.so.1"]));
    fx.image("p/_b.so", "p", &ElfSpec::module(&["libcache.so.1"]));
    let plan = fx.plan(false).expect("planned");
    assert_eq!(names(&plan), ["libcache.so.1"]);
}

/// A copied library is loaded at start-up, so a dependency of it that does
/// not resolve is refused naming it; the same dependency of an image loaded
/// on import is left to the loader.
#[test]
fn a_copied_library_with_an_unresolved_dependency_is_refused() {
    let mut fx = Fixture::new("native_linux_unresolved");
    let outside = fx.outside();
    fx.image(
        "pa/_a.so",
        "pa",
        &ElfSpec::module(&["libnope.so.1"]).runpath(&outside),
    );
    assert!(fx.plan(false).expect("planned").natives.is_empty());
    fx.write(
        "outside/libnat1.so.1",
        &ElfSpec::library("libnat1.so.1", &["libnope.so.1"]),
    );
    fx.image(
        "pa/_b.so",
        "pa",
        &ElfSpec::module(&["libnat1.so.1"]).runpath(&outside),
    );
    let err = fx.plan(false).expect_err("refused");
    assert!(
        err.contains("the native library `lib/libnat1.so.1`"),
        "{err}"
    );
    assert!(err.contains("needs `libnope.so.1`"), "{err}");
    assert!(err.contains("does not resolve on this machine"), "{err}");

    // A library vendored from the prefix is held to the same rule.
    let fx = Fixture::new("native_linux_unresolved_prefix");
    let rpath = fx.layout.prefix.join("lib").display().to_string();
    fx.write(
        "prefix/lib/libz.so.1",
        &ElfSpec::library("libz.so.1", &["libnope.so.1"]),
    );
    let zlib = ElfSpec::module(&["libz.so.1"]).runpath(&rpath);
    std::fs::write(
        fx.layout
            .dynload()
            .join("zlib.cpython-314-x86_64-linux-gnu.so"),
        elf_bytes(&zlib),
    )
    .unwrap();
    let err = fx.plan(true).expect_err("refused");
    assert!(err.contains("`libz.so.1` needs `libnope.so.1`"), "{err}");
}

/// A library of an unlocked distribution in a site directory inside the
/// prefix (a scanned one, or `<stdlib>/site-packages`) is a recorded
/// native when a closure image needs it, and stays a prefix vendor when an
/// interpreter image needs it (#1259).
#[test]
fn a_site_directory_library_inside_the_prefix_is_a_native_for_closure_images() {
    let mut fx = Fixture::new("native_linux_site");
    let stdlib_site = fx.layout.stdlib().join("site-packages").join("u");
    let scanned = fx.layout.prefix.join("lib").join("scanned");
    fx.write(
        "prefix/lib/python3.14/site-packages/u/libu.so.1",
        &ElfSpec::library("libu.so.1", &[]),
    );
    fx.write(
        "prefix/lib/scanned/libs.so.1",
        &ElfSpec::library("libs.so.1", &[]),
    );
    let search = format!("{}:{}", stdlib_site.display(), scanned.display());
    let module = ElfSpec::module(&["libu.so.1", "libs.so.1"]).runpath(&search);
    fx.image("pa/_a.so", "pa", &module);
    let mut closure = LockedClosure::of_files(fx.files.clone());
    closure.sites = vec![scanned];
    let planned = plan(
        &fx.layout.probe,
        Some(&closure),
        &fx.env,
        false,
        LibpythonLink::Shared,
    )
    .expect("planned");
    assert_eq!(names(&planned), ["libs.so.1", "libu.so.1"]);

    let fx = Fixture::new("native_linux_site_interpreter");
    fx.write(
        "prefix/lib/python3.14/site-packages/u/libu.so.1",
        &ElfSpec::library("libu.so.1", &[]),
    );
    let search = fx.layout.stdlib().join("site-packages/u");
    let search = search.display().to_string();
    let module = ElfSpec::module(&["libu.so.1"]).runpath(&search);
    let extension = fx
        .layout
        .dynload()
        .join("_u.cpython-314-x86_64-linux-gnu.so");
    std::fs::write(extension, elf_bytes(&module)).unwrap();
    let planned = fx.plan(true).expect("planned");
    assert_eq!(vendor_names(&planned), ["libu.so.1"]);
    assert!(planned.natives.is_empty());
}

/// A closure image that reaches a locked payload file through an absolute
/// `DT_RUNPATH` rather than `$ORIGIN` gets a native copy of it in `lib/`:
/// Linux has no rebind, so only an `$ORIGIN` payload match is kept (#1259).
#[test]
fn a_payload_file_reached_through_an_absolute_runpath_is_a_native() {
    let mut fx = Fixture::new("native_linux_payload_runpath");
    let search = fx.root.join("site/pq").display().to_string();
    let module = ElfSpec::module(&["libq.so.1"]).runpath(&search);
    fx.image("pq/_q.so", "pq", &module);
    fx.image("pq/libq.so.1", "pq", &ElfSpec::library("libq.so.1", &[]));
    let mut closure = LockedClosure::of_files(fx.files.clone());
    closure.sites = vec![fx.root.join("site")];
    let planned = plan(
        &fx.layout.probe,
        Some(&closure),
        &fx.env,
        false,
        LibpythonLink::Shared,
    )
    .expect("planned");
    assert_eq!(names(&planned), ["libq.so.1"]);
    assert_eq!(planned.natives[0].locked.required_by, ["pq"]);
    assert_eq!(vendor_names(&planned), ["libq.so.1"]);
}

/// A distribution Python's site directory lies under a system directory
/// (`/usr/lib/python3/dist-packages`): an unlocked distribution's library
/// there is still a native, not a kept system library, while the image's
/// own `$ORIGIN` payload sibling and a real system library stay kept
/// (#1259).
#[test]
fn a_site_directory_under_a_system_directory_still_yields_natives() {
    let mut fx = Fixture::new("native_linux_system_site");
    let site = fx.root.join("sys/python3/dist-packages");
    fx.write(
        "sys/python3/dist-packages/unlocked/libd.so.1",
        &ElfSpec::library("libd.so.1", &[]),
    );
    let search = format!("$ORIGIN:{}", site.join("unlocked").display());
    let module = ElfSpec::module(&["libp.so.1", "libd.so.1", "libc.so.6"]).runpath(&search);
    for (rel, spec) in [
        ("pd/_d.so", module),
        ("pd/libp.so.1", ElfSpec::library("libp.so.1", &[])),
    ] {
        let source = fx.write(&format!("sys/python3/dist-packages/{rel}"), &spec);
        fx.files.push(ClosureFile {
            rel: rel.to_string(),
            source,
            digest: String::new(),
            package: "pd".to_string(),
        });
    }
    let mut closure = LockedClosure::of_files(fx.files.clone());
    closure.sites = vec![site];
    let planned = plan(
        &fx.layout.probe,
        Some(&closure),
        &fx.env,
        false,
        LibpythonLink::Shared,
    )
    .expect("planned");
    assert_eq!(names(&planned), ["libd.so.1"]);
    // Only the native is copied: the payload sibling and libc are kept.
    assert_eq!(vendor_names(&planned), ["libd.so.1"]);
}
