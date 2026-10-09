//! The one definition of "the conformance harness sources" that every
//! text-reader of the harness shares.
//!
//! `tests/conformance.rs` is the crate root of the conformance harness, and its
//! cohort submodules live at `tests/conformance/*.rs` (declared from the root
//! with `#[path]`, since an integration-test crate root does not resolve a bare
//! `mod foo;` to a sibling directory). Two Rust guards —
//! `tests/conformance_matrix_guard.rs` and `tests/conformance_oracle_guard.rs`
//! — and `scripts/check_conformance_breadth.py` audit that harness as *text*;
//! if any one of them read only the root file, a test moved into a cohort file
//! would silently drop out of that reader's audit while every gate stayed
//! green. This module is `#[path]`-included by both Rust guards so they cannot
//! disagree about what the harness is; the Python checker's `read_harness`
//! deliberately mirrors the same rule.
//!
//! Contract (see the decision record named in `docs/TESTING.md`): the root
//! file first, then every `tests/conformance/*.rs` (extension exactly `rs`,
//! non-recursive, sorted by file name), each preceded by a newline, with
//! `\r\n` normalised to `\n` *after* concatenation. A missing module directory
//! yields the root alone. The root-first order is part of the contract: the
//! oracle guard's mutation controls rewrite the *first* occurrence of a
//! pattern in the concatenation, so every reader must see the same text in
//! the same order.
//!
//! Lives under `tests/harness_support/` rather than `tests/` so that Cargo's
//! `tests/*.rs` auto-discovery does not turn it into a test crate of its own.

use std::path::Path;

/// The conformance harness sources under `repo_root`, per the module contract.
pub fn harness_sources_in(repo_root: &Path) -> String {
    let root_file = repo_root.join("tests").join("conformance.rs");
    let mut text = std::fs::read_to_string(&root_file)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", root_file.display()));

    let module_dir = repo_root.join("tests").join("conformance");
    if let Ok(entries) = std::fs::read_dir(&module_dir) {
        let mut modules: Vec<_> = entries
            .map(|entry| {
                entry
                    .unwrap_or_else(|e| panic!("cannot read {}: {e}", module_dir.display()))
                    .path()
            })
            .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "rs"))
            .collect();
        modules.sort();
        for module in modules {
            text.push('\n');
            text.push_str(
                &std::fs::read_to_string(&module)
                    .unwrap_or_else(|e| panic!("cannot read {}: {e}", module.display())),
            );
        }
    }

    text.replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::harness_sources_in;
    use pycc_scratch::ScratchDir;
    use std::path::Path;

    /// A synthetic repository under a scratch root. `ScratchDir::new` plants
    /// its own liveness-marker file at the scratch root, so the synthetic tree
    /// is built one level down (`<scratch>/tests/...`) and the scratch root
    /// itself is never globbed.
    fn synthetic_repo(root_text: &str, modules: &[(&str, &str)]) -> ScratchDir {
        let scratch = ScratchDir::new("conformance_sources").expect("scratch dir");
        let tests = scratch.join("tests");
        std::fs::create_dir_all(&tests).expect("tests/");
        std::fs::write(tests.join("conformance.rs"), root_text).expect("root");
        if !modules.is_empty() {
            let dir = tests.join("conformance");
            std::fs::create_dir_all(&dir).expect("tests/conformance/");
            for (name, text) in modules {
                let path = dir.join(name);
                if let Some(parent) = Path::new(name).parent() {
                    std::fs::create_dir_all(dir.join(parent)).expect("nested dir");
                }
                std::fs::write(path, text).expect("module");
            }
        }
        scratch
    }

    #[test]
    fn a_root_without_a_module_directory_is_returned_alone() {
        let repo = synthetic_repo("fn root() {}\n", &[]);
        assert_eq!(harness_sources_in(&repo), "fn root() {}\n");
    }

    #[test]
    fn modules_follow_the_root_in_sorted_file_name_order() {
        let repo = synthetic_repo(
            "fn root() {}\n",
            &[
                ("b.rs", "fn b() { \"tests/fixtures/only_in_b.py\"; }\n"),
                ("a.rs", "fn a() {}\n"),
            ],
        );
        let text = harness_sources_in(&repo);
        assert_eq!(
            text,
            "fn root() {}\n\nfn a() {}\n\nfn b() { \"tests/fixtures/only_in_b.py\"; }\n"
        );
        assert!(text.contains("tests/fixtures/only_in_b.py"));
    }

    #[test]
    fn only_direct_rs_files_in_the_module_directory_count() {
        let repo = synthetic_repo(
            "fn root() {}\n",
            &[
                ("a.rs", "fn a() {}\n"),
                ("fixture.py", "print('tests/fixtures/not_a_module.py')\n"),
                (
                    "py30/nested.rs",
                    "fn nested() { \"tests/fixtures/nested.py\"; }\n",
                ),
            ],
        );
        let text = harness_sources_in(&repo);
        assert_eq!(text, "fn root() {}\n\nfn a() {}\n");
        assert!(!text.contains("not_a_module.py"));
        assert!(!text.contains("nested.py"));
    }

    #[test]
    fn crlf_line_endings_are_normalised_after_concatenation() {
        let repo = synthetic_repo("fn root() {}\r\n", &[("a.rs", "fn a() {}\r\n")]);
        let text = harness_sources_in(&repo);
        assert_eq!(text, "fn root() {}\n\nfn a() {}\n");
        assert!(!text.contains('\r'));
    }
}
