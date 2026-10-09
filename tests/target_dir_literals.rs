//! Mechanical gate for #629: no Rust source in this workspace may hard-code
//! a `target/`-shaped artifact path.
//!
//! Cargo's target directory is redirectable (`CARGO_TARGET_DIR`), and this
//! project's own coverage job redirects it. Any code that locates a
//! Cargo-produced artifact must go through
//! `pycc_codegen::artifact_layout::resolve_cargo_target_root` instead of
//! joining `"target"` onto a manifest directory; see
//! `docs/decisions/D-183-honor-cargo-target-dir-when-locating-build.md`.
//!
//! Scope: every `.rs` file reachable from `src/`, `crates/`, `tests/`, and
//! `benches/`. That list *is* the scope -- it is written out rather than
//! discovered by walking the workspace root, because under the coverage
//! job the workspace root contains a `target` symlink into the isolated
//! artifact tree, and walking it would descend into build output. `git
//! ls-files` is likewise unusable: the coverage job runs as `nobody` under
//! `env -i` with no `git` on `PATH`.
//!
//! `//` line-comment tails are stripped before matching, so prose that
//! *names* the old layout (`tests/nbody_bench.rs`, this file's own module
//! docs) is not an offender. Two deliberate limits on that, both of them
//! lexical rather than a parse: stripping from the first `//` also
//! truncates at a `//` inside a string literal, which can only ever
//! under-match; and `/* ... */` block comments are not recognized at all,
//! so a matching literal inside one *would* be reported. The workspace
//! contains no such block comment today, and the failure mode is a loud,
//! obviously-wrong offender line rather than a silent miss -- extend
//! `code_part` if block comments ever arrive in this scope.

/// Every `.rs` file in the scanned scope, discovered with an explicit work
/// list rather than recursion so the traversal has no untaken arm.
fn rust_sources_in_scope() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut pending: Vec<std::path::PathBuf> = ["src", "crates", "tests", "benches"]
        .iter()
        .map(|dir| root.join(dir))
        .collect();
    let rs = std::ffi::OsStr::new("rs");
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        } else if path.extension() == Some(rs) {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// The line with any `//` comment tail removed.
fn code_part(line: &str) -> &str {
    match line.find("//") {
        Some(at) => &line[..at],
        None => line,
    }
}

/// Every hard-coded target-directory path in one file's text, as
/// `<label>:<line>: <code>` strings.
///
/// A pure function over the text, exercised directly by the unit tests
/// below with both an offending and a clean sample. That is what keeps the
/// offending arm covered under D-014: driven only from the real tree the
/// arm would never run, since the tree is (and must stay) clean.
fn offenders_in(label: &str, text: &str) -> Vec<String> {
    // Assembled from fragments so this file does not match itself, which
    // would make the gate permanently red and force the very allowlisting
    // it exists to avoid.
    let needles = [
        concat!("target", "/"),
        concat!(".join(", "\"", "target", "\")"),
    ];
    let mut offenders = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let code = code_part(line);
        for needle in needles {
            if code.contains(needle) {
                offenders.push(format!("{label}:{}: {}", index + 1, code.trim()));
            }
        }
    }
    offenders
}

#[test]
fn no_rust_source_hard_codes_a_cargo_target_directory_path() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders: Vec<String> = Vec::new();
    for path in rust_sources_in_scope() {
        let text = std::fs::read_to_string(&path).unwrap();
        let label = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        offenders.extend(offenders_in(&label, &text));
    }
    assert_eq!(
        offenders,
        Vec::<String>::new(),
        "hard-coded Cargo target-directory paths found; resolve artifacts \
         through pycc_codegen::artifact_layout::resolve_cargo_target_root \
         instead (see docs/decisions/D-183-*.md)"
    );
}

#[test]
fn a_profile_directory_join_is_reported_with_its_location() {
    let sample = format!("let d = root.join({q}{t}/debug{q});", q = '"', t = "target");
    assert_eq!(
        offenders_in("sample.rs", &sample),
        vec![format!("sample.rs:1: {}", sample.trim())]
    );
}

#[test]
fn a_bare_target_segment_join_is_reported() {
    let sample = format!("let d = manifest.join({q}target{q});", q = '"');
    assert_eq!(offenders_in("sample.rs", &sample).len(), 1);
}

#[test]
fn resolver_based_code_and_prose_are_not_reported() {
    let prose = format!("// the old layout was {t}/debug", t = "target");
    let sample = format!(
        "{prose}\nlet root = resolve_cargo_target_root(manifest, lookup);\nlet d = root.join(profile);"
    );
    assert_eq!(offenders_in("sample.rs", &sample), Vec::<String>::new());
}

#[test]
fn the_scan_covers_this_workspace_s_rust_sources() {
    // Guards the scope list itself: if a directory is renamed away, the
    // gate would silently pass over an empty file set.
    let files = rust_sources_in_scope();
    assert!(files.len() > 20, "unexpectedly few sources: {files:?}");
    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    for expected in ["main.rs", "lib.rs", "slice0.rs", "check_bench.rs"] {
        assert!(names.contains(&expected.to_string()), "missing {expected}");
    }
}

#[test]
fn a_comment_tail_is_not_scanned_but_the_code_before_it_is() {
    let line = format!("let x = 1; // {t}/debug", t = "target");
    assert_eq!(code_part(&line), "let x = 1; ");
    assert_eq!(code_part("let x = 1;"), "let x = 1;");
}
