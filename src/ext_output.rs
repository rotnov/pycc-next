//! The `ext` output contract: what `pycc build PATH -o OUT --ext` writes and
//! what the artifact's module is called.
//!
//! `docs/CLI_SPEC.md:23-63` is the canonical statement of this contract
//! (D-244 rule 1); this module implements that text and nothing beyond it.
//! In short: the recognized extension suffixes are the **target** platform's
//! own `importlib.machinery.EXTENSION_SUFFIXES`, most-specific first; a
//! version-agnostic one already on `OUT` is honored as written, an
//! interpreter-specific tagged one is rejected, and otherwise the stable-ABI
//! spelling is appended. The module name is the basename with the *first*
//! matching suffix removed, which is how CPython's own finder derives a name
//! from the same file, and it becomes the `<mod>` of the exported
//! `PyInit_<mod>`.
//!
//! Two properties are deliberate and load-bearing:
//!
//! * **Pure.** No filesystem access, no environment reads, no process
//!   spawning: the inputs are the `OUT` path and the target platform, and the
//!   output is a resolved artifact path plus module name, or a typed error.
//!   That is what makes every arm of the contract reachable from an ordinary
//!   unit test on a single host.
//! * **Target-parameterised, never host-parameterised.** `pycc build` has
//!   `--target`, so the suffix set is selected from [`ExtPlatform`] derived
//!   from the *target* triple, never from `cfg!(windows)` of the building
//!   host. A host-conditioned suffix set would silently emit a Linux-shaped
//!   name for a Windows target, and would leave one arm unexecutable in CI.
//!
//! `try_build` calls [`resolve`] directly (`src/main.rs`'s `plan_ext`) —
//! `OUT` and the resolved target in, artifact path and module name out.
//! This module landed one pull request ahead of that caller (PR 1a of
//! #1036) so the contract was settled and unit-tested on its own; PR 1b
//! wired the flag and removed the `dead_code` allowance that ahead-of-caller
//! landing needed.
//!
//! Extracted into its own file rather than added to `src/main.rs`, which is
//! already near `AGENTS.md`'s ~1,000-line decomposition threshold; see
//! `src/frontend.rs:15-17` for the same precedent.
//!
//! **Not** a rule here: Python keywords are not rejected. `CLI_SPEC.md`
//! requires "a valid ASCII Python identifier", and `"class".isidentifier()`
//! is `True` in CPython, so rejecting keywords would be a rule this contract
//! does not state.

use std::path::{Path, PathBuf};

/// The target platform's extension-suffix family.
///
/// CPython's `EXTENSION_SUFFIXES` differs only between the POSIX shared-object
/// world (Linux and macOS, `.abi3.so` / `.so`) and Windows (`.pyd`), so the
/// two variants are the whole of what this contract needs to distinguish.
pub(crate) enum ExtPlatform {
    /// Linux and macOS: `.abi3.so`, then `.so`.
    Unix,
    /// Windows: `.pyd`.
    Windows,
}

impl ExtPlatform {
    /// The version-agnostic recognized suffixes, most-specific first. The
    /// interpreter-specific tagged suffix that heads CPython's own list is
    /// handled by [`has_tagged_suffix`] instead, because this contract
    /// rejects it rather than honoring it.
    fn recognized_suffixes(&self) -> &'static [&'static str] {
        match self {
            Self::Unix => &[".abi3.so", ".so"],
            Self::Windows => &[".pyd"],
        }
    }

    /// The stable-ABI spelling appended when `OUT` names no recognized
    /// suffix, and the spelling a tagged-suffix diagnostic points at.
    fn stable_suffix(&self) -> &'static str {
        match self {
            Self::Unix => ".abi3.so",
            Self::Windows => ".pyd",
        }
    }

    /// The dynamic-library suffix a tagged name ends with on this platform.
    fn tagged_tail(&self) -> &'static str {
        match self {
            Self::Unix => ".so",
            Self::Windows => ".pyd",
        }
    }
}

/// A resolved `--ext` output: where the artifact is written and what the
/// module inside it is called.
#[derive(Debug)]
pub(crate) struct ExtOutput {
    /// `OUT` as given when it already named a recognized suffix, otherwise
    /// `OUT` with the platform's stable-ABI suffix appended.
    pub(crate) artifact: PathBuf,
    /// The `<mod>` of the exported `PyInit_<mod>`. A valid ASCII Python
    /// identifier that is not `__init__`.
    pub(crate) module_name: String,
}

/// Why an `OUT` path cannot name an extension module.
#[derive(Debug, PartialEq)]
pub(crate) enum ExtOutputError {
    /// `OUT` has no file-name component at all (`/`, `..`).
    NoFileName {
        /// `OUT` as given, for the diagnostic.
        out: String,
    },
    /// `OUT` carries the interpreter-specific tagged suffix, which pins the
    /// artifact to one interpreter build instead of the stable ABI.
    TaggedSuffix {
        /// `OUT`'s basename, for the diagnostic.
        basename: String,
        /// The stable-ABI spelling to use instead.
        stable: &'static str,
    },
    /// The derived module name is not a valid ASCII Python identifier.
    InvalidModuleName {
        /// The name as derived, for the diagnostic.
        name: String,
    },
    /// The derived module name is `__init__`, a packaging shape `ext` mode
    /// does not claim.
    PackageInit {
        /// `OUT`'s basename, for the diagnostic.
        basename: String,
    },
}

impl ExtOutputError {
    /// The user-facing text, rendered by the caller as
    /// `eprintln!("error: {}", err.message())` per `src/main.rs`'s
    /// convention. The CLI attaches no diagnostic code to invocation errors.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::NoFileName { out } => {
                format!("--ext output path `{out}` has no file name to derive a module name from")
            }
            Self::TaggedSuffix { basename, stable } => format!(
                "--ext output path `{basename}` names an interpreter-specific extension suffix; \
                 ext mode builds one stable-ABI artifact per platform (D-244 rule 1), which a \
                 version-tagged filename hides from every other host -- use `{stable}` instead"
            ),
            Self::InvalidModuleName { name } => format!(
                "--ext module name `{name}`, derived from the output path, is not a valid ASCII \
                 Python identifier; it is the `<mod>` of the exported `PyInit_<mod>`, so an \
                 artifact is importable only under the name its own output path spells"
            ),
            Self::PackageInit { basename } => format!(
                "--ext output path `{basename}` derives the module name `__init__`; CPython's \
                 finder takes a package's name from the parent directory and looks for \
                 `PyInit_<package>`, a name this contract does not derive -- ext mode builds one \
                 extension module per artifact, not a package `__init__`"
            ),
        }
    }
}

/// Resolves `OUT` against the `ext` output contract for `platform`.
///
/// Implements `docs/CLI_SPEC.md:23-63`. The tagged-suffix rejection runs
/// before suffix stripping, so it reports its own diagnostic rather than
/// surfacing later as an identifier failure.
pub(crate) fn resolve(out: &Path, platform: &ExtPlatform) -> Result<ExtOutput, ExtOutputError> {
    let Some(file_name) = out.file_name() else {
        return Err(ExtOutputError::NoFileName {
            out: out.display().to_string(),
        });
    };
    // Lossy is exact for this contract's purposes: invalid UTF-8 becomes
    // U+FFFD, which is not ASCII, so a non-UTF-8 basename is rejected by the
    // identifier rule it definitionally fails. Every path that reaches the
    // artifact below has already proven the basename is ASCII.
    let basename = file_name.to_string_lossy();

    if has_tagged_suffix(&basename, platform) {
        return Err(ExtOutputError::TaggedSuffix {
            basename: basename.into_owned(),
            stable: platform.stable_suffix(),
        });
    }

    let (module_name, artifact) = match platform
        .recognized_suffixes()
        .iter()
        .find_map(|suffix| basename.strip_suffix(suffix))
    {
        Some(stem) => (stem.to_string(), out.to_path_buf()),
        None => {
            let mut appended = file_name.to_os_string();
            appended.push(platform.stable_suffix());
            (basename.to_string(), out.with_file_name(appended))
        }
    };

    if !is_ascii_identifier(&module_name) {
        return Err(ExtOutputError::InvalidModuleName { name: module_name });
    }
    if module_name == "__init__" {
        return Err(ExtOutputError::PackageInit {
            basename: basename.into_owned(),
        });
    }

    Ok(ExtOutput {
        artifact,
        module_name,
    })
}

/// Whether `basename` ends with the platform's interpreter-specific tagged
/// suffix (`m.cpython-314-x86_64-linux-gnu.so`, `m.cp314-win_amd64.pyd`).
///
/// Recognized by shape rather than by asking an interpreter: this module is
/// pure, and the tag is what CPython's `EXTENSION_SUFFIXES` head always looks
/// like -- the dot-segment before the dynamic-library suffix is `cpython-`
/// plus the version and platform on POSIX, and `cp` plus the version digits
/// plus the platform on Windows. Both tags contain hyphens and underscores
/// but no further dots, so the segment split is unambiguous.
fn has_tagged_suffix(basename: &str, platform: &ExtPlatform) -> bool {
    let Some(rest) = basename.strip_suffix(platform.tagged_tail()) else {
        return false;
    };
    let Some((_, segment)) = rest.rsplit_once('.') else {
        return false;
    };
    let tag_prefix = match platform {
        ExtPlatform::Unix => "cpython-",
        ExtPlatform::Windows => "cp",
    };
    segment
        .strip_prefix(tag_prefix)
        .is_some_and(|tail| tail.starts_with(|c: char| c.is_ascii_digit()))
}

/// Whether `name` is a valid ASCII Python identifier. ASCII specifically:
/// CPython loads a non-ASCII-named module through `PyInitU_<punycode>`, which
/// this contract does not emit, so a non-ASCII name is rejected rather than
/// encoded. Keywords are not rejected -- `"class".isidentifier()` is `True`.
fn is_ascii_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod ext_output_tests {
    use super::*;

    /// Resolves `out` for `platform`, asserting success, and returns the
    /// artifact path and module name for comparison.
    ///
    /// The path comes back as a [`PathBuf`], never as a rendered string: a
    /// `Path`'s rendering carries the *host*'s separator, so `dist/m` renders
    /// as `dist\\m` on Windows and a string comparison would assert the build
    /// host rather than the contract.
    fn ok(out: &str, platform: &ExtPlatform) -> (PathBuf, String) {
        let resolved = resolve(Path::new(out), platform).expect("expected a resolved ext output");
        (resolved.artifact, resolved.module_name)
    }

    /// Resolves `out` for `platform`, asserting rejection, and returns the
    /// error for variant comparison.
    fn err(out: &str, platform: &ExtPlatform) -> ExtOutputError {
        resolve(Path::new(out), platform).expect_err("expected a rejected ext output")
    }

    #[test]
    fn unix_appends_the_stable_abi_suffix_when_out_names_none() {
        assert_eq!(
            ok("m", &ExtPlatform::Unix),
            (PathBuf::from("m.abi3.so"), "m".to_string())
        );
    }

    #[test]
    fn a_resolved_output_carries_both_halves_of_the_contract() {
        let resolved =
            resolve(Path::new("m"), &ExtPlatform::Unix).expect("expected a resolved ext output");
        let rendered = format!("{resolved:?}");
        assert!(rendered.contains("m.abi3.so"), "got: {rendered}");
        assert!(rendered.contains("\"m\""), "got: {rendered}");
    }

    #[test]
    fn unix_appends_beside_the_out_directory() {
        assert_eq!(
            ok("dist/m", &ExtPlatform::Unix),
            (Path::new("dist").join("m.abi3.so"), "m".to_string())
        );
    }

    #[test]
    fn unix_honors_a_bare_so_suffix_as_written() {
        assert_eq!(
            ok("m.so", &ExtPlatform::Unix),
            (PathBuf::from("m.so"), "m".to_string())
        );
    }

    #[test]
    fn unix_strips_the_first_matching_suffix_not_only_the_final_one() {
        // `.abi3.so` precedes `.so` in the ordered list, so the name is `m`
        // -- what CPython's finder derives -- and never `m.abi3`.
        assert_eq!(
            ok("m.abi3.so", &ExtPlatform::Unix),
            (PathBuf::from("m.abi3.so"), "m".to_string())
        );
    }

    #[test]
    fn unix_accepts_a_leading_underscore_name() {
        assert_eq!(
            ok("_m", &ExtPlatform::Unix),
            (PathBuf::from("_m.abi3.so"), "_m".to_string())
        );
    }

    #[test]
    fn unix_rejects_the_interpreter_specific_tagged_suffix() {
        assert_eq!(
            err("m.cpython-314-x86_64-linux-gnu.so", &ExtPlatform::Unix),
            ExtOutputError::TaggedSuffix {
                basename: "m.cpython-314-x86_64-linux-gnu.so".to_string(),
                stable: ".abi3.so",
            }
        );
    }

    #[test]
    fn unix_requires_version_digits_after_the_cpython_tag_prefix() {
        // `cpython-` alone is not the tag shape; the dot in the derived name
        // is the real defect, so the identifier rule reports it.
        assert_eq!(
            err("m.cpython-nightly.so", &ExtPlatform::Unix),
            ExtOutputError::InvalidModuleName {
                name: "m.cpython-nightly".to_string()
            }
        );
    }

    #[test]
    fn unix_rejects_a_non_ascii_derived_name() {
        // CPython would import this only as `PyInitU_md_5ja`.
        assert_eq!(
            err("mód", &ExtPlatform::Unix),
            ExtOutputError::InvalidModuleName {
                name: "mód".to_string()
            }
        );
    }

    #[test]
    fn unix_rejects_a_derived_name_starting_with_a_digit() {
        assert_eq!(
            err("9m", &ExtPlatform::Unix),
            ExtOutputError::InvalidModuleName {
                name: "9m".to_string()
            }
        );
    }

    #[test]
    fn unix_rejects_an_unrecognized_suffix_that_survives_into_the_name() {
        // `.foo` is not a recognized suffix, so `.abi3.so` is appended and
        // the name is derived from the appended basename: `m.foo`.
        assert_eq!(
            err("m.foo", &ExtPlatform::Unix),
            ExtOutputError::InvalidModuleName {
                name: "m.foo".to_string()
            }
        );
    }

    #[test]
    fn unix_rejects_a_basename_that_is_only_a_recognized_suffix() {
        assert_eq!(
            err(".abi3.so", &ExtPlatform::Unix),
            ExtOutputError::InvalidModuleName {
                name: String::new()
            }
        );
    }

    #[test]
    fn unix_rejects_a_bare_dynamic_suffix_with_no_preceding_segment() {
        assert_eq!(
            err(".so", &ExtPlatform::Unix),
            ExtOutputError::InvalidModuleName {
                name: String::new()
            }
        );
    }

    #[test]
    fn unix_rejects_a_package_init_basename_separately() {
        assert_eq!(
            err("pkg/__init__.abi3.so", &ExtPlatform::Unix),
            ExtOutputError::PackageInit {
                basename: "__init__.abi3.so".to_string()
            }
        );
    }

    #[test]
    fn an_out_path_with_no_file_name_is_rejected() {
        assert_eq!(
            err("..", &ExtPlatform::Unix),
            ExtOutputError::NoFileName {
                out: "..".to_string()
            }
        );
    }

    #[test]
    fn windows_appends_pyd_when_out_names_none() {
        assert_eq!(
            ok("m", &ExtPlatform::Windows),
            (PathBuf::from("m.pyd"), "m".to_string())
        );
    }

    #[test]
    fn windows_honors_a_pyd_suffix_as_written() {
        assert_eq!(
            ok("m.pyd", &ExtPlatform::Windows),
            (PathBuf::from("m.pyd"), "m".to_string())
        );
    }

    #[test]
    fn windows_rejects_the_interpreter_specific_tagged_suffix() {
        assert_eq!(
            err("m.cp314-win_amd64.pyd", &ExtPlatform::Windows),
            ExtOutputError::TaggedSuffix {
                basename: "m.cp314-win_amd64.pyd".to_string(),
                stable: ".pyd",
            }
        );
    }

    #[test]
    fn windows_does_not_mistake_an_ordinary_segment_for_a_tag() {
        assert_eq!(
            err("m.foo.pyd", &ExtPlatform::Windows),
            ExtOutputError::InvalidModuleName {
                name: "m.foo".to_string()
            }
        );
    }

    #[test]
    fn windows_requires_version_digits_after_the_cp_tag_prefix() {
        assert_eq!(
            err("m.cpx.pyd", &ExtPlatform::Windows),
            ExtOutputError::InvalidModuleName {
                name: "m.cpx".to_string()
            }
        );
    }

    #[test]
    fn windows_does_not_recognize_the_posix_suffix_set() {
        // `.so` is not a Windows extension suffix, so it is not stripped;
        // `.pyd` is appended and the leftover `.so` fails the name rule.
        assert_eq!(
            err("m.so", &ExtPlatform::Windows),
            ExtOutputError::InvalidModuleName {
                name: "m.so".to_string()
            }
        );
    }

    #[test]
    fn the_suffix_set_follows_the_target_triple_not_the_host() {
        assert!(matches!(
            crate::ext_build::ExtLinkPlatform::from_target_triple("x86_64-pc-windows-msvc")
                .suffix_platform(),
            ExtPlatform::Windows
        ));
        assert!(matches!(
            crate::ext_build::ExtLinkPlatform::from_target_triple("x86_64-unknown-linux-gnu")
                .suffix_platform(),
            ExtPlatform::Unix
        ));
        assert!(matches!(
            crate::ext_build::ExtLinkPlatform::from_target_triple("aarch64-apple-darwin")
                .suffix_platform(),
            ExtPlatform::Unix
        ));
    }

    #[test]
    fn every_rejection_renders_a_distinct_message_and_debug_form() {
        let errors = [
            ExtOutputError::NoFileName {
                out: "/".to_string(),
            },
            ExtOutputError::TaggedSuffix {
                basename: "m.cpython-314-x86_64-linux-gnu.so".to_string(),
                stable: ".abi3.so",
            },
            ExtOutputError::InvalidModuleName {
                name: "m.foo".to_string(),
            },
            ExtOutputError::PackageInit {
                basename: "__init__.abi3.so".to_string(),
            },
        ];
        let messages: Vec<String> = errors.iter().map(ExtOutputError::message).collect();
        for message in &messages {
            assert!(message.starts_with("--ext "), "got: {message}");
        }
        assert_eq!(
            messages.len(),
            messages
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            "each rejection must render its own diagnostic"
        );
        for error in &errors {
            assert!(!format!("{error:?}").is_empty());
        }
    }
}
