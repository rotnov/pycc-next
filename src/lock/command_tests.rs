use super::*;
use crate::embed::fake_layout::fake_layout;
use crate::interop_policy::InteropCli;
use pycc_scratch::ScratchDir;
use schema::LockedNative;

const HOST: (&str, &str) = ("aarch64", "macos");

fn program(dir: &Path, body: &str) -> std::path::PathBuf {
    let path = dir.join("m.py");
    std::fs::write(&path, body).unwrap();
    path
}

fn layout_dir(dir: &Path) -> std::path::PathBuf {
    let layout = dir.join("layout");
    std::fs::create_dir_all(&layout).unwrap();
    layout
}

fn absent(dir: &Path) -> EmbedToolchain {
    EmbedToolchain::with_interpreter(dir.join("absent-python"))
}

fn env_message(result: Result<(), LockFailure>) -> String {
    match result {
        Err(LockFailure::Env(message)) => message,
        Err(LockFailure::Stale(message)) => panic!("stale, not env: {message}"),
        Err(LockFailure::Frontend(_)) => panic!("frontend, not env"),
        Ok(()) => panic!("succeeded"),
    }
}

fn section(entry: &str) -> LockTarget {
    LockTarget {
        entry: entry.to_string(),
        triple: "aarch64-apple-darwin".to_string(),
        python: "3.14.7".to_string(),
        cache_tag: "cpython-314".to_string(),
        platform: "macosx-11.0-arm64".to_string(),
        libpython_sha256: "aa".to_string(),
        roots: vec!["tinypkg".to_string()],
        optional_roots: Vec::new(),
        package: vec![LockedPackage {
            name: "tinypkg".to_string(),
            version: "1.0".to_string(),
            site: "purelib".to_string(),
            files: 2,
            tree_sha256: "bb".to_string(),
            requires: Vec::new(),
        }],
        native: Vec::new(),
    }
}

/// A host with no Tier-1 triple is refused before the program is read. A
/// Windows host locks (#1296): `src/embed/windows_lock_tests.rs`.
#[test]
fn a_non_tier_1_host_is_refused_before_the_program_is_read() {
    let dir = ScratchDir::new("lock_hosts").unwrap();
    let missing = dir.join("missing.py");
    let other = run_lock_on(
        &missing,
        false,
        InteropCli::default(),
        &absent(&dir),
        ("riscv64", "linux"),
    );
    assert!(env_message(other).contains("not one"));
}

#[test]
fn the_frontend_refusal_and_every_failure_class_map_to_their_exit_codes() {
    let dir = ScratchDir::new("lock_frontend").unwrap();
    let missing = dir.join("missing.py");
    // A fixed Tier-1 host: on Windows `run_lock` refuses before the frontend.
    let failure =
        run_lock_on(&missing, false, InteropCli::default(), &absent(&dir), HOST).unwrap_err();
    assert!(matches!(failure, LockFailure::Frontend(_)));
    assert_eq!(report_lock_failure(failure), 2);
    assert_eq!(report_lock_failure(LockFailure::Env("e".to_string())), 2);
    assert_eq!(report_lock_failure(LockFailure::Stale("s".to_string())), 1);
}

#[test]
fn an_entry_key_joins_components_with_slashes_and_refuses_non_utf8() {
    assert_eq!(entry_key(Path::new("app/main.py")).unwrap(), "app/main.py");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bad = Path::new(std::ffi::OsStr::from_bytes(b"app/\xff.py"));
        assert!(entry_key(bad).unwrap_err().contains("not valid UTF-8"));
    }
}

#[test]
fn an_unreadable_existing_lock_is_refused() {
    let dir = ScratchDir::new("lock_unreadable").unwrap();
    let path = program(&dir, "print(1)\n");
    std::fs::create_dir_all(dir.join("pycc.lock").join("inner")).unwrap();
    let result = run_lock_on(&path, false, InteropCli::default(), &absent(&dir), HOST);
    assert!(env_message(result).contains("cannot read"));
}

#[test]
fn a_failed_rename_removes_the_temporary_file() {
    let scratch = ScratchDir::new("lock_rename").unwrap();
    let dir = scratch.join("app");
    std::fs::create_dir_all(dir.join("pycc.lock").join("inner")).unwrap();
    let err = write_atomically(&dir, "pycc.lock", "text").unwrap_err();
    assert!(err.contains("cannot write"), "{err}");
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["pycc.lock"]);
}

#[cfg(unix)]
#[test]
fn a_lock_that_cannot_be_removed_is_an_environment_failure() {
    use std::os::unix::fs::PermissionsExt;
    let dir = ScratchDir::new("lock_remove").unwrap();
    let app = dir.join("app");
    std::fs::create_dir_all(&app).unwrap();
    let path = program(&app, "print(1)\n");
    let lock = Lock {
        version: LOCK_VERSION,
        target: vec![section("m.py")],
    };
    std::fs::write(app.join("pycc.lock"), schema::render(&lock)).unwrap();
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = run_lock_on(&path, false, InteropCli::default(), &absent(&dir), HOST);
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(env_message(result).contains("cannot remove"));
}

#[test]
fn a_lock_in_another_form_fails_check() {
    let dir = ScratchDir::new("lock_form").unwrap();
    let path = program(&dir, "print(1)\n");
    let mut other = section("m.py");
    other.triple = "x86_64-pc-windows-msvc".to_string();
    let lock = Lock {
        version: LOCK_VERSION,
        target: vec![other],
    };
    let text = schema::render(&lock).replace("# Generated by `pycc lock`. Do not edit.\n", "");
    std::fs::write(dir.join("pycc.lock"), text).unwrap();
    match run_lock_on(&path, true, InteropCli::default(), &absent(&dir), HOST) {
        Err(LockFailure::Stale(message)) => {
            assert!(message.contains("not in the form"), "{message}")
        }
        _ => panic!("expected a stale lock"),
    }
}

#[test]
fn an_interpreter_that_fails_either_probe_is_refused() {
    let dir = ScratchDir::new("lock_probes").unwrap();
    let path = program(&dir, "import json\n");
    let result = run_lock_on(&path, false, InteropCli::default(), &absent(&dir), HOST);
    assert!(env_message(result).contains("could not run the embed interpreter"));
    let layout = fake_layout(&layout_dir(&dir));
    let toolchain = EmbedToolchain::with_probe(dir.join("absent-python"), layout.probe);
    let result = run_lock_on(&path, false, InteropCli::default(), &toolchain, HOST);
    assert!(env_message(result).contains("could not run the lock interpreter"));
}

#[cfg(unix)]
#[test]
fn an_unreadable_libpython_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = ScratchDir::new("lock_libpython").unwrap();
    let path = program(&dir, "import json\n");
    let layout = fake_layout(&layout_dir(&dir));
    let lines = probe::tests::probe_lines(&dir, &dir);
    let script = probe::tests::fake_interpreter(&dir, &format!("cat <<'PYCC'\n{lines}PYCC\n"));
    std::fs::set_permissions(layout.library(), std::fs::Permissions::from_mode(0o000)).unwrap();
    let toolchain = EmbedToolchain::with_probe(script, layout.probe.clone());
    let result = run_lock_on(&path, false, InteropCli::default(), &toolchain, HOST);
    std::fs::set_permissions(layout.library(), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(env_message(result).contains("libpython3.14.dylib"));
}

#[test]
fn a_stale_reason_names_what_changed() {
    let old = section("m.py");
    assert!(stale_reason(None, Some(&old), 0).contains("no section"));
    assert!(stale_reason(Some(&old), None, 0).contains("not needed"));
    assert!(stale_reason(Some(&old), Some(&old), 2).contains("2 section(s)"));
    assert!(stale_reason(None, None, 0).contains("not in the form"));
    let mut new = old.clone();
    new.platform = "linux-x86_64".to_string();
    assert_eq!(
        stale_reason(Some(&old), Some(&new), 0),
        "`platform` is `macosx-11.0-arm64` in the lock but `linux-x86_64` in the environment"
    );
}

#[test]
fn the_first_difference_covers_roots_packages_and_natives() {
    let old = section("m.py");
    let mut roots = old.clone();
    roots.roots.push("other".to_string());
    assert!(first_difference(&old, &roots).starts_with("`roots`"));
    let mut optional = old.clone();
    optional.optional_roots.push("fastpkg".to_string());
    assert_eq!(
        first_difference(&old, &optional),
        "`optional-roots` is [] in the lock but [\"fastpkg\"] for the program"
    );
    let mut added = old.clone();
    added.package.push(LockedPackage {
        name: "tinydep".to_string(),
        ..old.package[0].clone()
    });
    assert!(first_difference(&old, &added).contains("`tinydep` is in the closure but not locked"));
    assert!(first_difference(&added, &old).contains("`tinydep` is locked but not in the closure"));
    let mut changed = old.clone();
    changed.package[0].tree_sha256 = "cc".to_string();
    assert!(first_difference(&old, &changed).contains("`tinypkg` differs"));
    let mut native = old.clone();
    native.native.push(LockedNative {
        name: "libfoo.dylib".to_string(),
        sha256: "dd".to_string(),
        required_by: vec!["tinypkg".to_string()],
    });
    assert!(first_difference(&native, &old).contains("`libfoo.dylib` is locked but not needed"));
    assert!(first_difference(&old, &native).contains("`libfoo.dylib` is needed but not locked"));
    let mut other = native.clone();
    other.native.insert(
        0,
        LockedNative {
            name: "libbar.dylib".to_string(),
            ..native.native[0].clone()
        },
    );
    let mut swapped = other.clone();
    swapped.native.reverse();
    assert_eq!(
        first_difference(&other, &swapped),
        "its entries are in a different order"
    );
}

/// #1291: a foreign import nested in a module-level block is a direct root
/// exactly like a top-level one; the lock does not read the site.
#[test]
fn a_block_foreign_import_is_a_direct_root() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: Vec::new(),
        type_aliases: Vec::new(),
        imports: vec![ImportBinding::Foreign {
            local_name: "np".to_string(),
            module_path: "numpy".to_string(),
            from: None,
            site: pycc_hir::ForeignImportSite::Block { optional: false },
            span: pycc_diag::Span::new(0, 0),
        }],
        class_defs: Vec::new(),
    };
    assert_eq!(
        direct_roots(&hir).into_iter().collect::<Vec<_>>(),
        vec!["numpy".to_string()]
    );
}

/// #1290: a root imported only through `optional` block sites is optional,
/// one imported anywhere unguarded is required, and a standard-library
/// root is neither; every one of them is still a direct root.
#[test]
fn split_roots_classifies_optional_roots_and_required_wins() {
    let foreign = |module_path: &str, optional: Option<bool>| ImportBinding::Foreign {
        local_name: module_path.to_string(),
        module_path: module_path.to_string(),
        from: None,
        site: match optional {
            Some(optional) => pycc_hir::ForeignImportSite::Block { optional },
            None => pycc_hir::ForeignImportSite::Item(0),
        },
        span: pycc_diag::Span::new(0, 0),
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: Vec::new(),
        type_aliases: Vec::new(),
        imports: vec![
            foreign("fastpkg.speedups", Some(true)),
            foreign("bothpkg", Some(true)),
            foreign("bothpkg", None),
            foreign("json", Some(true)),
            foreign("tkinter", Some(true)),
            foreign("plainpkg", Some(false)),
            ImportBinding::Project {
                local_name: "helper".to_string(),
                module_path: "helper".to_string(),
                kind: pycc_hir::ProjectBindingKind::Function,
            },
        ],
        class_defs: Vec::new(),
    };
    let split = split_roots(&hir);
    assert_eq!(split.required(), ["bothpkg", "plainpkg"]);
    assert_eq!(split.optional(), ["fastpkg"]);
    assert_eq!(
        direct_roots(&hir).into_iter().collect::<Vec<_>>(),
        ["bothpkg", "fastpkg", "plainpkg"]
    );
}

/// #1485: an `except ImportError` fallback import is classified by the
/// guard *enclosing* its `try`, as every handler-level import is (#1290):
/// at module level it is required, while the body's import is optional.
/// Pinned so a change to the fallback's lock classification is deliberate.
#[test]
fn a_fallback_import_in_an_import_error_handler_is_a_required_root() {
    let source = "try:\n    import fastpkg as speed\nexcept ImportError:\n    \
                  import slowpkg as speed\n";
    let module = pycc_parser::parse(source).unwrap();
    let mut resolved = pycc_hir::ResolvedImports::default();
    for request in pycc_hir::project_import_requests(&module) {
        resolved.insert(request.span, pycc_hir::ResolvedImport::Foreign);
    }
    let hir = pycc_hir::lower_module(&module, &resolved, None)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"))
        .hir;
    let split = split_roots(&hir);
    assert_eq!(split.required(), ["slowpkg"]);
    assert_eq!(split.optional(), ["fastpkg"]);
}
