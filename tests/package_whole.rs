//! The whole-package build (v0.8, ROADMAP accepts 1 and 4): `pycc build PKG
//! --ext` compiles every module of the package directory into one
//! extension, `PKG/__init__.abi3.so`, whose finder imports each module as a
//! module of its own.
//!
//! The fixture `tests/fixtures/package_whole/pkw` covers what the package's
//! modules must keep from being separate extensions: import-time side
//! effects in import order, each module's `__name__`, `__file__`,
//! `__spec__`, `__package__` and `__path__`, a subpackage, relative and
//! absolute imports between the modules, circular imports, a PEP 562
//! `__getattr__`, `importlib.reload`, a module global rebound from Python,
//! pickling by reference (functions, classes, exceptions, instances), data
//! files read with `importlib.resources`,
//! classes subclassed from another module (`ABC`, `Generic`), an exception
//! class of one module caught as its builtin base by another module's
//! compiled `except`, calls of functions compiled in other modules (native
//! while the names bind them, through the objects once Python code rebinds
//! the names or reloads the module that defined them) -- and no leak. The oracle is CPython: `driver.py` runs
//! against the sources and against the compiled tree, and the outputs must
//! be byte-identical. The textkit fixture of `tests/package_ext.rs` is built
//! whole too.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// Copies the directory `from` to `to`, recursively.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create a directory");
    for entry in std::fs::read_dir(from).expect("read the fixture") {
        let entry = entry.expect("an entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("its type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy a file");
        }
    }
}

/// Removes every `.py` file under `dir`.
fn remove_sources(dir: &Path) {
    for entry in std::fs::read_dir(dir).expect("read the tree") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            remove_sources(&path);
        } else if path.extension().is_some_and(|ext| ext == "py") {
            std::fs::remove_file(&path).expect("remove a source");
        }
    }
}

/// The files under `dir`, relative, sorted.
fn files(dir: &Path, base: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("read the tree") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            files(&path, base, out);
        } else {
            out.push(
                path.strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    out.sort();
}

/// Builds the package `dir/name` whole, with `args` besides.
fn build_whole(dir: &Path, name: &str, args: &[&str]) -> Output {
    let package = dir.join(name);
    pycc()
        .arg("build")
        .arg(&package)
        .arg("-o")
        .arg(package.join("__init__"))
        .args(["--ext", "--non-strict", "--report-fallbacks"])
        .args(args)
        .output()
        .expect("pycc should spawn")
}

fn run(driver: &Path, tree: &Path) -> Output {
    python()
        .arg("-I")
        .arg(driver)
        .arg(tree)
        .current_dir(tree)
        .output()
        .expect("python3 should spawn")
}

/// The extension's file name on this platform.
fn artifact() -> &'static str {
    if cfg!(windows) {
        "__init__.pyd"
    } else {
        "__init__.abi3.so"
    }
}

/// Whether the host's `cc` really packs relative relocations: a shared
/// object it links under `-z pack-relative-relocs` has a `RELR` entry
/// (Ubuntu 24.04's GNU ld 2.42 packs on x86-64 and ignores the flag on
/// aarch64).
fn host_linker_packs() -> bool {
    let scratch = ScratchDir::new("package_whole_relr_probe").expect("scratch");
    let source = scratch.join("probe.c");
    std::fs::write(&source, "int a[2];\nint *p[2] = {&a[0], &a[1]};\n").expect("probe source");
    let object = scratch.join("probe.so");
    let linked = Command::new("cc")
        .args(["-shared", "-fPIC", "-Wl,-z,pack-relative-relocs"])
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .output()
        .expect("cc runs");
    if !linked.status.success() {
        return false;
    }
    let dynamic = Command::new("readelf").arg("-d").arg(&object).output().expect("readelf runs");
    text(&dynamic.stdout).contains("(RELR)")
}

/// v0.11: on a glibc that reads `DT_RELR` (2.36 and newer) with a linker
/// that packs them ([`host_linker_packs`]), the extension's relative
/// relocations are packed into one (`readelf -d` lists a `RELR` entry and
/// `readelf -r` a `.relr.dyn` section), and the extension depends on
/// `GLIBC_ABI_DT_RELR`; on any other glibc host it has no `RELR` entry.
fn assert_packed_relocations(extension: &Path) {
    if !cfg!(all(target_os = "linux", target_env = "gnu")) {
        return;
    }
    let readelf = |flag: &str| {
        let output = Command::new("readelf")
            .arg(flag)
            .arg(extension)
            .output()
            .expect("readelf runs");
        assert!(output.status.success(), "{}", text(&output.stderr));
        text(&output.stdout)
    };
    let ldd = Command::new("ldd").arg("--version").output().expect("ldd runs");
    let version = text(&ldd.stdout)
        .lines()
        .next()
        .and_then(|line| line.rsplit(' ').next())
        .map(str::to_string)
        .unwrap_or_default();
    let mut parts = version.split('.').map(|part| part.parse::<u32>().unwrap_or(0));
    let glibc = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    let dynamic = readelf("-d");
    if glibc < (2, 36) || !host_linker_packs() {
        assert!(!dynamic.contains("(RELR)"), "{dynamic}");
        return;
    }
    assert!(dynamic.contains("(RELR)"), "{dynamic}");
    assert!(readelf("-r").contains(".relr.dyn"));
    assert!(readelf("-V").contains("GLIBC_ABI_DT_RELR"));
}

fn the_package_matches_cpython(release: bool) {
    let built = ScratchDir::new("package_whole_built").expect("scratch");
    let plain = ScratchDir::new("package_whole_plain").expect("scratch");
    let fixture = fixtures().join("package_whole");
    copy_tree(&fixture.join("pkw"), &built.join("pkw"));
    copy_tree(&fixture.join("pkw"), &plain.join("pkw"));
    let build = build_whole(&built, "pkw", if release { &["--release"] } else { &[] });
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{}{report}", text(&pycc_scratch::lf(&build.stdout)));
    // Every module was built, as a member of the one extension.
    for module in [
        "pkw",
        "pkw.base",
        "pkw.sub",
        "pkw.sub.circle",
        "pkw.cyc_a",
        "pkw.state",
        "pkw.util",
        "pkw.calls",
        "pkw.cursor",
        "pkw.scan",
    ] {
        assert!(
            report.contains(&format!("pycc: {module}: built in ")),
            "{report}"
        );
    }
    // Calls of functions other modules compiled go to them natively.
    for (name, module) in [
        ("scaled", "pkw.util"),
        ("shout", "pkw.util"),
        ("_shout", "pkw.util"),
        ("note", "pkw.util"),
        ("fails", "pkw.util"),
        ("label", "pkw.base"),
        ("check", "pkw.base"),
    ] {
        assert!(
            report.contains(&format!(
                "native call: `{name}` calls `{module}`'s compiled function"
            )),
            "{report}"
        );
    }
    // A class another module subclasses through a third module's re-export
    // stays native since v0.9: the subclass is a CPython class over it,
    // whose overrides compiled calls run (`pycc_mir::dispatch`).
    assert!(
        !report.contains("of the package subclasses `Tag`"),
        "{report}"
    );
    assert!(report.contains("class `Bold` at"), "{report}");
    // Instances of a class another module compiled are read natively.
    assert!(
        report.contains("native class: instances of `pkw.cursor.Cursor` are read natively"),
        "{report}"
    );
    assert_packed_relocations(&built.join("pkw").join(artifact()));
    remove_sources(&built.join("pkw"));
    let mut left = Vec::new();
    files(&built.join("pkw"), &built.join("pkw"), &mut left);
    // One extension, and the package's data file.
    assert_eq!(left, [artifact(), "sub/data.txt"]);
    let driver = fixture.join("driver.py");
    let theirs = run(&driver, &plain);
    assert_eq!(theirs.status.code(), Some(0), "{}", text(&pycc_scratch::lf(&theirs.stderr)));
    let ours = run(&driver, &built);
    assert_eq!(
        text(&pycc_scratch::lf(&ours.stdout)),
        text(&pycc_scratch::lf(&theirs.stdout)),
        "{}",
        text(&pycc_scratch::lf(&ours.stderr))
    );
    assert_eq!(ours.status.code(), Some(0), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert!(text(&pycc_scratch::lf(&ours.stdout)).contains("leaks none"));
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_package_built_whole_matches_cpython() {
    the_package_matches_cpython(false);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_package_built_whole_with_release_matches_cpython() {
    the_package_matches_cpython(true);
}

/// The package of `tests/package_ext.rs`, built whole instead of module by
/// module, behaves the same.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_textkit_package_built_whole_matches_cpython() {
    let built = ScratchDir::new("package_whole_textkit").expect("scratch");
    let plain = ScratchDir::new("package_whole_textkit_plain").expect("scratch");
    let fixture = fixtures().join("ext_package");
    copy_tree(&fixture.join("textkit"), &built.join("textkit"));
    copy_tree(&fixture.join("textkit"), &plain.join("textkit"));
    let build = build_whole(&built, "textkit", &["--release"]);
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    remove_sources(&built.join("textkit"));
    let driver = fixture.join("driver.py");
    let theirs = run(&driver, &plain);
    let ours = run(&driver, &built);
    assert_eq!(theirs.status.code(), Some(0), "{}", text(&pycc_scratch::lf(&theirs.stderr)));
    assert_eq!(
        text(&pycc_scratch::lf(&ours.stdout)),
        text(&pycc_scratch::lf(&theirs.stdout)),
        "{}",
        text(&pycc_scratch::lf(&ours.stderr))
    );
}

/// `--exclude` keeps a module or subpackage in Python: it is not built, its
/// source is imported by CPython's own finder, and the compiled modules
/// import it as before.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_excluded_module_stays_python() {
    let dir = ScratchDir::new("package_whole_exclude").expect("scratch");
    let package = dir.join("pke");
    std::fs::create_dir_all(package.join("tests")).expect("create");
    std::fs::write(package.join("__init__.py"), "from . import core\n").expect("write");
    std::fs::write(
        package.join("core.py"),
        "def f(x: int) -> int:\n    return x + 1\n",
    )
    .expect("write");
    std::fs::write(package.join("tests").join("__init__.py"), "").expect("write");
    std::fs::write(
        package.join("tests").join("t.py"),
        "from pke import core\nRESULT = core.f(41)\n",
    )
    .expect("write");
    let build = build_whole(&dir, "pke", &["--exclude", "tests"]);
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    assert!(report.contains("pycc: pke.core: built in"), "{report}");
    assert!(!report.contains("pke.tests"), "{report}");
    std::fs::remove_file(package.join("__init__.py")).expect("remove");
    std::fs::remove_file(package.join("core.py")).expect("remove");
    let check = python()
        .args(["-I", "-c"])
        .arg(
            "import sys; sys.path.insert(0, sys.argv[1]); import pke.tests.t as t, pke.core as c; \
             print(t.RESULT, type(c.__spec__.loader).__name__, t.__file__.endswith('t.py'))",
        )
        .arg(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_eq!(
        text(&pycc_scratch::lf(&check.stdout)),
        "42 _PyccFinder True\n",
        "{}",
        text(&pycc_scratch::lf(&check.stderr))
    );
}

/// What a package build refuses, before compiling anything: a directory
/// that is no package, an `-o` that does not name the package's own
/// `__init__`, and excluding the package's `__init__`.
#[test]
fn a_package_build_refuses_what_it_cannot_build() {
    let dir = ScratchDir::new("package_whole_refusals").expect("scratch");
    let package = dir.join("pkr");
    std::fs::create_dir_all(&package).expect("create");
    std::fs::write(package.join("m.py"), "X = 1\n").expect("write");
    let run = |args: &[&std::ffi::OsStr]| {
        let out = pycc()
            .arg("build")
            .args(args)
            .arg("--ext")
            .output()
            .expect("spawn");
        assert_eq!(out.status.code(), Some(2));
        text(&pycc_scratch::lf(&out.stderr))
    };
    let init = package.join("__init__");
    let err = run(&[package.as_os_str(), "-o".as_ref(), init.as_os_str()]);
    assert!(err.contains("has no `__init__.py`"), "{err}");
    std::fs::write(package.join("__init__.py"), "X = 1\n").expect("write");
    let elsewhere = dir.join("other");
    let err = run(&[package.as_os_str(), "-o".as_ref(), elsewhere.as_os_str()]);
    assert!(err.contains("must name"), "{err}");
    let err = run(&[
        package.as_os_str(),
        "-o".as_ref(),
        init.as_os_str(),
        "--exclude".as_ref(),
        "pkr".as_ref(),
    ]);
    assert!(err.contains("cannot be excluded"), "{err}");
}

/// A member build's objects define no symbol another member could define too:
/// every global symbol of its LLVM object and of its C unit carries the
/// member's prefix (`pycc_codegen::member_prefix`), the C unit's own
/// definitions being `static` -- so a symbol that escaped the prefix (a
/// promoted local, a new shim entry point) fails here rather than as a
/// duplicate definition in some package's link.
#[cfg(unix)]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_member_build_defines_only_prefixed_symbols() {
    let dir = ScratchDir::new("package_whole_member").expect("scratch");
    let package = dir.join("p");
    std::fs::create_dir_all(&package).expect("create");
    std::fs::write(package.join("__init__.py"), "").expect("write");
    let source = package.join("m.py");
    let fixture = fixtures().join("package_whole").join("pkw").join("base.py");
    std::fs::copy(fixture, &source).expect("copy");
    let out = dir.join("member");
    std::fs::create_dir_all(&out).expect("create");
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&out)
        .args([
            "--ext",
            "--non-strict",
            "--release",
            "--foreign-relative-imports",
            "--package-member",
            "p.m",
            "--exception-tag-base",
            "40",
        ])
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    let manifest = std::fs::read_to_string(out.join("member.txt")).expect("the manifest");
    let objects: Vec<&str> = manifest
        .lines()
        .filter(|l| !l.starts_with("max_tag "))
        .collect();
    assert!(objects.len() >= 2, "{manifest}");
    // `BaseError` and `DeepError` take tags from the base.
    assert!(manifest.contains("max_tag 41\n"), "{manifest}");
    for object in objects {
        let nm = Command::new("nm")
            .arg("-g")
            .arg(object)
            .output()
            .expect("nm should spawn");
        assert!(nm.status.success(), "{}", text(&pycc_scratch::lf(&nm.stderr)));
        for line in text(&pycc_scratch::lf(&nm.stdout)).lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            // An undefined symbol has no address.
            let [_, kind, name] = fields[..] else {
                continue;
            };
            if kind == "U" {
                continue;
            }
            let name = match name.strip_prefix('_') {
                Some(rest) if cfg!(target_os = "macos") => rest,
                _ => name,
            };
            assert!(name.starts_with("pycc_P1p1m_"), "{object} defines {name}");
        }
    }
}

/// A member build compiles against the package build's probe of the
/// interpreter (`--ext-probe`) instead of starting the interpreter again: it
/// builds with no interpreter to start at all (its CPython source is then
/// embedded uncompiled), and a probe it cannot read is an environment
/// failure (exit 2) that names the file.
#[cfg(unix)]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_member_build_compiles_against_the_package_builds_probe() {
    let dir = ScratchDir::new("package_whole_probe").expect("scratch");
    let package = dir.join("p");
    std::fs::create_dir_all(&package).expect("create");
    std::fs::write(package.join("__init__.py"), "").expect("write");
    let source = package.join("m.py");
    std::fs::write(
        &source,
        "def f(x: int) -> int:\n    return x + 1\n\nclass C:\n    pass\n",
    )
    .expect("write");
    let probe = python()
        .args([
            "-I",
            "-c",
            "import os,sys,sysconfig\nprint('%d.%d' % sys.version_info[:2])\n\
             print(sysconfig.get_path('include'))\nprint(os.path.join(sys.base_prefix, 'libs'))",
        ])
        .output()
        .expect("python should spawn");
    assert!(probe.status.success(), "{}", text(&probe.stderr));
    let probe_file = dir.join("probe.txt");
    std::fs::write(&probe_file, &probe.stdout).expect("write the probe");
    let member = |probe: &Path| {
        let out = dir.join("member");
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).expect("create");
        pycc()
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(&out)
            .args([
                "--ext",
                "--non-strict",
                "--release",
                "--foreign-relative-imports",
                "--package-member",
                "p.m",
                "--member-threads",
                "1",
                "--ext-probe",
            ])
            .arg(probe)
            .env("PYCC_PYTHON", "/nonexistent/pycc-test-python")
            .env_remove("PYCC_PYTHON_INCLUDE")
            .output()
            .expect("pycc should spawn")
    };
    let built = member(&probe_file);
    assert!(built.status.success(), "{}", text(&built.stderr));
    assert!(dir.join("member").join("member.txt").is_file());
    let unreadable = member(&dir.join("absent.txt"));
    let err = text(&unreadable.stderr);
    assert_eq!(unreadable.status.code(), Some(2), "{err}");
    assert!(err.contains("absent.txt"), "{err}");
}
