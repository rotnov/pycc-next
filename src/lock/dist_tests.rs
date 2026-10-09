use super::*;
use pycc_scratch::ScratchDir;

#[test]
fn metadata_reads_headers_continuations_and_stops_at_the_body() {
    let text = "Metadata-Version: 2.1\r\nName: Tiny_Pkg\nversion: 1.0\nRequires-Dist: a\nRequires-Dist: b ;\n  sys_platform == 'x'\nSummary:\n\nRequires-Dist: body-not-a-header\n";
    let meta = parse_metadata(text).unwrap();
    assert_eq!(meta.name, "Tiny_Pkg");
    assert_eq!(meta.version, "1.0");
    assert_eq!(meta.requires_dist, ["a", "b ; sys_platform == 'x'"]);
}

#[test]
fn metadata_without_name_or_version_is_refused() {
    assert!(
        parse_metadata("Version: 1\n")
            .unwrap_err()
            .contains("`Name`")
    );
    assert!(
        parse_metadata("Name: x\nVersion:\n")
            .unwrap_err()
            .contains("`Version`")
    );
    assert!(
        parse_metadata("  orphan continuation\nno colon here\nName: x\n")
            .unwrap_err()
            .contains("`Version`")
    );
}

#[test]
fn record_reads_quoted_fields_and_doubled_quotes() {
    let text = "\"a,b.py\",sha256=AAAA,4\r\n\"q\"\"x.py\",,\n\npkg/../x.py,sha256=AA,1\npkg/__pycache__/m.cpython-314.pyc,,\nlast.py,sha256=AA,2";
    let rows = parse_record(text).unwrap();
    let paths: Vec<_> = rows.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "a,b.py",
            "q\"x.py",
            "pkg/../x.py",
            "pkg/__pycache__/m.cpython-314.pyc",
            "last.py"
        ]
    );
    assert_eq!(rows[0].hash, "sha256=AAAA");
    assert_eq!(rows[1].hash, "");
}

#[test]
fn malformed_record_rows_are_refused() {
    assert!(
        parse_record("a.py,sha256=AA\n")
            .unwrap_err()
            .contains("2 fields")
    );
    assert!(
        parse_record("\"a.py,x,1\n")
            .unwrap_err()
            .contains("unterminated")
    );
    assert!(parse_record(",x,1\n").unwrap_err().contains("empty path"));
    assert!(parse_record("a,b,c,d\n").unwrap_err().contains("4 fields"));
}

#[test]
fn urlsafe_base64_decodes_known_vectors_and_refuses_garbage() {
    assert_eq!(decode_urlsafe_b64("").unwrap(), b"");
    assert_eq!(decode_urlsafe_b64("Zg").unwrap(), b"f");
    assert_eq!(decode_urlsafe_b64("Zm8").unwrap(), b"fo");
    assert_eq!(decode_urlsafe_b64("Zm9v").unwrap(), b"foo");
    assert_eq!(decode_urlsafe_b64("Zm9vYg").unwrap(), b"foob");
    assert_eq!(decode_urlsafe_b64("-_8").unwrap(), [0xfb, 0xff]);
    assert!(decode_urlsafe_b64("Zm9v=").is_none());
    assert!(decode_urlsafe_b64("Zm9vY").is_none());
    assert!(decode_urlsafe_b64("+/8").is_none());
}

#[test]
fn names_root_matches_packages_modules_and_extensions_only() {
    for platform in [EmbedPlatform::MacOs, EmbedPlatform::Linux] {
        for yes in [
            "R/x.py",
            "R/sub/y.py",
            "R.py",
            "R.pyc",
            "R.so",
            "R.abi3.so",
            "R.cpython-314-darwin.so",
            "R.cpython-314-x86_64-linux-gnu.so",
        ] {
            assert!(names_root(yes, "R", platform), "{yes}");
        }
        for no in [
            "Rx.py",
            "R.dist-info/RECORD",
            "R.pth",
            "R/",
            "R..so",
            "x/R.py",
            "R.a/b.so",
            "R",
            "R.pyw",
            "R.pyd",
            "R.cp314-win_amd64.pyd",
        ] {
            assert!(!names_root(no, "R", platform), "{no}");
        }
    }
}

/// A Windows host's import suffixes (#1296): `.pyw` sources and `.pyd`
/// extensions, tagged or not, and never a POSIX `.so`.
#[test]
fn names_root_on_windows_matches_the_windows_import_suffixes() {
    let windows = EmbedPlatform::Windows;
    for yes in [
        "R/x.py",
        "R/sub/y.pyd",
        "R.py",
        "R.pyw",
        "R.pyc",
        "R.pyd",
        "R.cp314-win_amd64.pyd",
    ] {
        assert!(names_root(yes, "R", windows), "{yes}");
    }
    for no in [
        "Rx.py",
        "R.dist-info/RECORD",
        "R.pth",
        "R/",
        "R..pyd",
        "R.x/y.pyd",
        "R.so",
        "R.abi3.so",
        "R.dll",
        "R",
    ] {
        assert!(!names_root(no, "R", windows), "{no}");
    }
}

#[test]
fn scan_indexes_dist_infos_by_normalized_name() {
    let dir = ScratchDir::new("lock_dist_scan").unwrap();
    for name in [
        "Tiny_Pkg-1.0.dist-info",
        "nodash.dist-info",
        "other-2.dist-info",
        "pkg-1.0.egg-info",
    ] {
        std::fs::create_dir(dir.join(name)).unwrap();
    }
    std::fs::write(dir.join("file-1.dist-info"), "").unwrap();
    let site = Site {
        path: dir.to_path_buf(),
        kind: SiteKind::Platlib,
    };
    let found = scan_site(&site).unwrap();
    let names: Vec<_> = found
        .iter()
        .map(|d| (d.name.as_str(), d.dir_name.as_str()))
        .collect();
    // Sorted by directory name bytes: `T` sorts before `o`.
    assert_eq!(
        names,
        [
            ("tiny-pkg", "Tiny_Pkg-1.0.dist-info"),
            ("other", "other-2.dist-info")
        ]
    );
    assert_eq!(found[1].path(), dir.join("other-2.dist-info"));
    assert!(found[1].display().ends_with("other-2.dist-info"));
    assert_eq!(SiteKind::Platlib.as_str(), "platlib");
    let missing = Site {
        path: dir.join("absent"),
        kind: SiteKind::Purelib,
    };
    let err = scan_site(&missing).unwrap_err();
    assert!(
        err.contains("purelib site directory") && err.contains("absent"),
        "{err}"
    );
}

#[test]
fn lexical_normalization_resolves_dots() {
    assert_eq!(
        normalize_lexically(Path::new("/a/b/./c/../../d")),
        PathBuf::from("/a/d")
    );
    // `components()` keeps only a leading `.`; it is dropped too.
    assert_eq!(
        normalize_lexically(Path::new("./x/../y")),
        PathBuf::from("y")
    );
}
