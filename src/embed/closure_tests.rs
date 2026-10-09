//! The closure copy against hand-made plans: each refusal of
//! `copy_closure` and the sidecar listing, with no lock and no interpreter.

use super::*;
use crate::embed::sha256::sha256_hex;
use crate::lock::build::ClosureFile;
use pycc_scratch::ScratchDir;
use std::path::PathBuf;

fn source(dir: &Path, rel: &str, bytes: &[u8]) -> ClosureFile {
    let path = dir.join("site").join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    ClosureFile {
        rel: rel.to_string(),
        source: path,
        digest: sha256_hex(bytes),
        package: "pkg".to_string(),
    }
}

fn staging(dir: &Path) -> PathBuf {
    let staging = dir.join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    staging
}

#[test]
fn every_file_is_copied_and_mach_o_images_are_reported() {
    let dir = ScratchDir::new("closure_copy").unwrap();
    let files = vec![
        source(&dir, "pkg/__init__.py", b"X = 1\n"),
        source(&dir, "pkg/_ext.so", &[0xcf, 0xfa, 0xed, 0xfe, 0, 0]),
        source(
            &dir,
            "pkg/Main.class",
            &[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 0x34],
        ),
    ];
    let staging = staging(&dir);
    let images = copy_closure(&LockedClosure::of_files(files), &staging).unwrap();
    assert_eq!(images, ["pkg/_ext.so"]);
    let root = staging.join(CLOSURE_DIR);
    assert_eq!(
        std::fs::read(root.join("pkg/__init__.py")).unwrap(),
        b"X = 1\n"
    );
    let mut copied: Vec<String> = std::fs::read_dir(root.join("pkg"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    copied.sort();
    assert_eq!(copied, ["Main.class", "__init__.py", "_ext.so"]);
}

#[cfg(unix)]
#[test]
fn a_copy_keeps_the_source_mode_plus_owner_write() {
    use std::os::unix::fs::PermissionsExt;
    let dir = ScratchDir::new("closure_mode").unwrap();
    let tool = source(&dir, "pkg/bin/tool", b"#!/bin/sh\n");
    std::fs::set_permissions(&tool.source, std::fs::Permissions::from_mode(0o555)).unwrap();
    let staging = staging(&dir);
    copy_closure(&LockedClosure::of_files(vec![tool]), &staging).unwrap();
    let copied = std::fs::metadata(staging.join("closure/pkg/bin/tool")).unwrap();
    assert_eq!(copied.permissions().mode() & 0o777, 0o755);
}

#[test]
fn a_file_changed_since_it_was_installed_is_refused() {
    let dir = ScratchDir::new("closure_changed").unwrap();
    let mut file = source(&dir, "pkg/__init__.py", b"X = 1\n");
    file.digest = sha256_hex(b"X = 2\n");
    let err = copy_closure(&LockedClosure::of_files(vec![file]), &staging(&dir)).unwrap_err();
    assert!(
        err.contains("of distribution `pkg` does not match its RECORD hash"),
        "{err}"
    );
    assert!(err.contains("reinstall the distribution"), "{err}");
    assert!(err.contains("run `pycc lock m.py`"), "{err}");
}

#[test]
fn a_missing_source_is_an_environment_failure() {
    let dir = ScratchDir::new("closure_missing").unwrap();
    let file = source(&dir, "pkg/__init__.py", b"X = 1\n");
    std::fs::remove_file(&file.source).unwrap();
    let err = copy_closure(&LockedClosure::of_files(vec![file]), &staging(&dir)).unwrap_err();
    assert!(err.contains("could not read"), "{err}");
}

#[test]
fn a_destination_that_already_exists_unclaimed_is_an_environment_failure() {
    let dir = ScratchDir::new("closure_exists").unwrap();
    let file = source(&dir, "pkg/x", b"x");
    let staging = staging(&dir);
    std::fs::create_dir_all(staging.join("closure/pkg/x")).unwrap();
    let err = copy_closure(&LockedClosure::of_files(vec![file]), &staging).unwrap_err();
    assert!(err.contains("could not create"), "{err}");
}

/// Two payload paths that differ only in case: on a case-insensitive file
/// system the second would overwrite the first, so it is refused naming
/// both; on a case-sensitive one both are copied.
#[test]
fn two_paths_differing_only_in_case_never_overwrite_each_other() {
    let dir = ScratchDir::new("closure_case").unwrap();
    let probe = dir.join("CaseProbe");
    std::fs::write(&probe, "").unwrap();
    let insensitive = dir.join("caseprobe").exists();
    let upper = source(&dir, "pkg/X.py", b"same\n");
    let mut lower = ClosureFile {
        rel: "pkg/x.py".to_string(),
        ..upper.clone()
    };
    lower.package = "other".to_string();
    let staging = staging(&dir);
    let result = copy_closure(&LockedClosure::of_files(vec![upper, lower]), &staging);
    if insensitive {
        let err = result.unwrap_err();
        assert!(
            err.contains(
                "`pkg/x.py` of distribution `other` and `pkg/X.py` of distribution `pkg` \
                 name the same file on this case-insensitive file system"
            ),
            "{err}"
        );
    } else {
        result.unwrap();
        assert!(staging.join("closure/pkg/x.py").is_file());
        assert!(staging.join("closure/pkg/X.py").is_file());
    }
}
