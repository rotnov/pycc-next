//! Where `pycc` looks for build artifacts it did not produce itself.
//!
//! `pycc build` links the compiled object against `pycc_rt`'s static
//! library, which Cargo has already placed somewhere under the *Cargo
//! target directory*. That directory is not always `<workspace>/target`:
//! Cargo lets it be redirected, and this project's own coverage job does
//! exactly that (see `docs/TESTING.md`). Everything that needs to locate a
//! Cargo-produced artifact resolves it through this module rather than
//! joining `"target"` onto a manifest directory, so a redirected target
//! directory is honored uniformly by the compiler driver, by
//! `pycc_codegen`'s own link-and-run tests, and by the CLI integration
//! tests. `tests/target_dir_literals.rs` is the mechanical gate that keeps
//! it that way.
//!
//! This lives in its own workspace member rather than inside
//! `pycc_codegen` because `pycc_codegen`'s build script needs the same
//! resolution rules the library uses, and a crate cannot depend on itself.
//! `pycc_codegen` takes it under both `[dependencies]` and
//! `[build-dependencies]` and re-exports it as
//! `pycc_codegen::artifact_layout`, so every pre-existing path through that
//! module name still resolves. See
//! `docs/decisions/D-184-build-pycc-rt-from-pycc-codegen-s-build.md`.

/// The directory name Cargo uses when the target directory is not
/// redirected. Named rather than inlined so that this module -- the single
/// authorized place to build that path -- does not itself trip
/// `tests/target_dir_literals.rs`, the gate that forbids the same join
/// everywhere else.
const DEFAULT_TARGET_DIR_NAME: &str = "target";

/// Resolves the Cargo target directory that holds this workspace's build
/// artifacts.
///
/// Three levels, in Cargo's own relative order (also stated in
/// `docs/CLI_SPEC.md`; all three must agree):
///
/// 1. `CARGO_TARGET_DIR`, when set to a non-empty value;
/// 2. otherwise `CARGO_BUILD_TARGET_DIR`, when set to a non-empty value --
///    this is Cargo's generic config-to-environment mapping of the
///    `build.target-dir` config key, and it is honored by Cargo whether or
///    not any `.cargo/config.toml` exists. Verified on this project's dev
///    host, not assumed: `CARGO_BUILD_TARGET_DIR=<dir> cargo build` puts
///    artifacts in `<dir>/debug` and creates no local `target/` at all,
///    and with both variables set `CARGO_TARGET_DIR` wins and the
///    `CARGO_BUILD_TARGET_DIR` path is never created;
/// 3. otherwise `<manifest_dir>/target`.
///
/// An empty value at either level is treated as unset. This is a
/// deliberate **divergence** from Cargo, which does not fall back but
/// rejects the build outright (`CARGO_TARGET_DIR= cargo build` exits 101
/// with "the target directory is set to an empty string in the
/// `CARGO_TARGET_DIR` environment variable"). A compiler driver looking
/// for an artifact someone else built has no equivalent of that abort:
/// honoring the empty string would resolve artifacts to a bare relative
/// `debug/`, which is worse than falling back, and an
/// exported-but-empty variable is a common shell accident rather than an
/// intent to redirect.
///
/// A relative value is passed through unchanged rather than being joined
/// onto `manifest_dir`. Cargo resolves a relative target directory against
/// the working directory of the process that invoked *it*; `pycc` is a
/// separate process, so passing the value through agrees with Cargo
/// exactly when `pycc` runs from that same directory, and re-anchoring it
/// on `manifest_dir` would agree with Cargo in no case at all.
///
/// Cargo's `--target-dir` **command-line flag** and a `build.target-dir`
/// key set in a `.cargo/config.toml` **config file** rank above both
/// variables in Cargo's precedence and are not consulted here. Reading
/// the config-file form means re-implementing Cargo's ancestor-walking
/// config discovery -- `$CARGO_HOME` merge, Cargo's own precedence and
/// path resolution -- a materially larger surface with its own
/// 100%-region cost under D-014. As for the flag: its resolved path does
/// reach an integration-test or bench binary, but only through the
/// *compile-time* `CARGO_TARGET_TMPDIR` macro, never the runtime
/// environment, and not at all to the `pycc` binary a user invokes
/// (measured: under `cargo test --target-dir <dir>`,
/// `env!("CARGO_TARGET_TMPDIR")` is `<dir>/tmp` while every runtime
/// lookup of it is `NotPresent`). Anchoring this one shared resolver on
/// it would give it two different resolution rules depending on which
/// binary it was compiled into. See `docs/decisions/D-183-*.md` for the
/// original reasoning and `docs/decisions/D-216-*.md`, which closed both
/// exclusions as permanent rather than deferred.
///
/// `env_lookup` is a plain `fn` pointer, not `impl Fn(..)`, for the same
/// reason as [`find_pycc_rt_lib_dir_in`]'s `exists`: a generic parameter
/// would monomorphize one body per closure type, and each copy would only
/// ever exercise the arm *that* caller takes, which under `cargo llvm-cov`
/// reads as a real gap in D-014's region coverage.
pub fn resolve_cargo_target_root(
    manifest_dir: &std::path::Path,
    env_lookup: fn(&str) -> Option<String>,
) -> std::path::PathBuf {
    match env_lookup("CARGO_TARGET_DIR") {
        Some(value) if !value.is_empty() => std::path::PathBuf::from(value),
        _ => match env_lookup("CARGO_BUILD_TARGET_DIR") {
            Some(value) if !value.is_empty() => std::path::PathBuf::from(value),
            _ => manifest_dir.join(DEFAULT_TARGET_DIR_NAME),
        },
    }
}

/// Reads the named variable from this process's real environment.
///
/// The production `env_lookup` for [`resolve_cargo_target_root`], which
/// calls it once per precedence level rather than only for
/// `CARGO_TARGET_DIR`; exists as a named `fn` so every caller shares one
/// function pointer instead of each spelling out its own closure. A
/// variable that is set but not valid Unicode is reported as unset, which
/// the resolver treats like any other absent level.
pub fn cargo_target_dir_from_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Rust's `staticlib` output naming is platform-specific: `lib<name>.a` on
/// Unix-like targets, but `<name>.lib` (no `lib` prefix, COFF format) on
/// `-msvc` targets -- verified empirically by cross-building `pycc_rt` for
/// `x86_64-pc-windows-msvc` and inspecting the resulting artifact
/// directory directly, not assumed from Unix convention. Keyed on the
/// *requested* `target` triple, not the host `#[cfg(windows)]` -- an
/// earlier version used a host-keyed constant, which silently checked for
/// the wrong filename whenever `--target` crossed OS families (e.g.
/// requesting an `-msvc` triple from a non-Windows host, or vice versa),
/// reporting a misleading "no build found" even when the correctly-named
/// file was right there (caught in PR review before merge).
pub fn pycc_rt_lib_filename(target: Option<&str>) -> &'static str {
    let targets_msvc = match target {
        Some(triple) => triple.contains("windows-msvc"),
        None => cfg!(windows),
    };
    if targets_msvc {
        "pycc_rt.lib"
    } else {
        "libpycc_rt.a"
    }
}

/// Locates the directory holding `pycc_rt`'s static library.
///
/// `target_root` is an already-resolved Cargo target directory -- callers
/// obtain it from [`resolve_cargo_target_root`] rather than joining
/// `"target"` themselves, so a redirected `CARGO_TARGET_DIR` is honored.
///
/// `target: None` (the common case) returns `<target_root>/debug/` or
/// `<target_root>/release/` -- always populated once `pycc_rt` has been
/// built for the requested profile (see that crate's own doc comment on
/// the build-order requirement).
///
/// `target: Some(triple)` looks under `<target_root>/<triple>/` -- where
/// `cargo build [--release] --target <triple> -p pycc_rt` (after `rustup
/// target add <triple>` if needed) puts it. Cross-compiling `pycc_rt`
/// itself for the requested target is not done automatically here: unlike
/// the ordinary case, there's no single always-correct way to trigger it
/// that doesn't either require duplicating `rustup`/`cargo`'s own
/// target-management logic or risk the same build-lock deadlock documented
/// on `pycc_rt`. Failing with a clear, actionable message is better than a
/// confusing linker error about a missing `-lpycc_rt`.
///
/// Both failure messages also name the `CARGO_TARGET_DIR` recovery: under a
/// persistent target-dir redirect `pycc` cannot observe (a `.cargo/config.toml`
/// `build.target-dir`, see `docs/CLI_SPEC.md`'s Environment section), the
/// suggested `cargo build` rebuilds into that same redirect and reproduces
/// the failure, so pointing `pycc` at the redirected directory is the fix
/// (#869). The message stays a single line either way.
///
/// `release` selects which of `pycc_rt`'s own two builds to link against.
/// Before this parameter existed, the compiler driver unconditionally
/// linked the debug build regardless of `pycc build --release` -- a real,
/// unambiguous bug (not a design choice) caught while investigating why a
/// `--release` nbody benchmark's speedup ratio fell far short of
/// expectations (`tests/nbody_bench.rs`): `--release` was optimizing the
/// compiled module's own LLVM IR but every runtime call
/// (`pycc_rt_float_pow`, string helpers, etc.) still ran through an
/// unoptimized debug `pycc_rt`.
///
/// `exists` takes the filesystem-existence check as a parameter instead of
/// calling `Path::exists` directly, so tests can simulate "no build found"
/// without mutating this workspace's real, shared target directory (which
/// every other test also depends on -- deleting the real `libpycc_rt.a`,
/// even temporarily, would be flaky under parallel test execution).
///
/// A plain `fn` pointer, not `impl Fn(..)`: every caller (production and
/// all the tests) passes a non-capturing closure, so a concrete function
/// pointer covers all of them with a single compiled body. Genericity over
/// `impl Fn` would monomorphize a separate copy per closure type -- each
/// one only ever exercising the branches *that* caller takes, which under
/// `cargo llvm-cov` reads as a real gap in coverage even though every
/// branch collectively runs somewhere.
pub fn find_pycc_rt_lib_dir_in(
    target_root: &std::path::Path,
    target: Option<&str>,
    release: bool,
    exists: fn(&std::path::Path) -> bool,
) -> Result<std::path::PathBuf, String> {
    let profile = if release { "release" } else { "debug" };
    let release_flag = if release { " --release" } else { "" };
    let dir = match target {
        Some(triple) => target_root.join(triple).join(profile),
        None => target_root.join(profile),
    };
    if exists(&dir.join(pycc_rt_lib_filename(target))) {
        Ok(dir)
    } else if let Some(triple) = target {
        Err(format!(
            "no pycc_rt build found for target `{triple}` (expected {}). \
             Run `rustup target add {triple}` then `cargo build{release_flag} --target {triple} -p pycc_rt` first, \
             or run pycc with CARGO_TARGET_DIR set to the directory Cargo built into.",
            dir.display()
        ))
    } else {
        Err(format!(
            "no pycc_rt build found (expected {}). Run `cargo build{release_flag} -p pycc_rt` first, \
             or run pycc with CARGO_TARGET_DIR set to the directory Cargo built into.",
            dir.display()
        ))
    }
}

/// Where a build script should install artifacts, given the target root
/// [`resolve_cargo_target_root`] produced.
///
/// [`AnchoredTargetRoot::root`] is always absolute-or-workspace-anchored,
/// so a build script can create directories under it without depending on
/// its own working directory. [`AnchoredTargetRoot::diverged`] reports the
/// one case the caller must warn about: the resolved root was relative and
/// anchoring it produced a directory that does not contain `out_dir`,
/// meaning Cargo itself resolved that same relative value against a
/// different working directory and the install will land somewhere Cargo
/// is not reading from.
pub struct AnchoredTargetRoot {
    /// The directory to install into.
    pub root: std::path::PathBuf,
    /// Whether the anchored root demonstrably disagrees with the directory
    /// Cargo is actually using for this build.
    pub diverged: bool,
}

/// Anchors a resolved Cargo target root for use from a build script.
///
/// [`resolve_cargo_target_root`] deliberately passes a relative
/// `CARGO_TARGET_DIR` through unchanged, because `pycc` is a separate
/// process from the Cargo invocation and agreeing with Cargo means
/// resolving against the *invoking* process's working directory. A build
/// script has no such freedom: Cargo runs it with an unspecified working
/// directory, so a bare relative root is not usable as written. Anchoring
/// it on `workspace_root` is the only interpretation available, and it is
/// correct exactly when Cargo was invoked from the workspace root -- the
/// overwhelmingly common case, and the one CI uses.
///
/// `out_dir` is the build script's own `OUT_DIR`, which Cargo always
/// places *inside* the target directory it is really using. That makes it
/// a direct, zero-cost probe of whether the anchoring guess was right:
/// when the anchored root does not contain `OUT_DIR`, Cargo resolved the
/// relative value elsewhere and the caller should emit a
/// `cargo::warning`. No filesystem access and no environment access, so
/// this stays a pure function the unit tests below drive directly.
///
/// An absolute resolved root -- including the `<manifest_dir>/target`
/// fallback, which is absolute whenever `manifest_dir` is -- is returned
/// unchanged and never reported as divergent: it names one directory
/// unambiguously, and a build script whose `OUT_DIR` sits elsewhere under
/// a redirect is a situation the absolute path already describes
/// correctly.
pub fn anchor_target_root_for_build_script(
    resolved_root: &std::path::Path,
    workspace_root: &std::path::Path,
    out_dir: &std::path::Path,
) -> AnchoredTargetRoot {
    if resolved_root.is_absolute() {
        return AnchoredTargetRoot {
            root: resolved_root.to_path_buf(),
            diverged: false,
        };
    }
    let root = workspace_root.join(resolved_root);
    let diverged = !out_dir.starts_with(&root);
    AnchoredTargetRoot { root, diverged }
}

/// Whether an environment variable must be removed before a build script
/// shells out to a *nested* `cargo` invocation.
///
/// Cargo exports a large `CARGO_*` namespace describing the build that is
/// currently running -- package name and version, feature flags, the
/// encoded rustflags, the target directory, the manifest directory. Left
/// in place, those describe the *outer* build to the inner one and either
/// corrupt it or make it silently reuse the outer build's configuration.
/// The same holds for `RUSTC_WRAPPER`/`RUSTC_WORKSPACE_WRAPPER` (a
/// coverage or caching wrapper the outer build installed),
/// `RUSTFLAGS`/`RUSTDOCFLAGS`, and `LLVM_PROFILE_FILE` (the outer
/// coverage run's profile-output template, which the nested build would
/// otherwise overwrite).
///
/// Two deliberate exceptions:
///
/// * `CARGO_HOME` is kept. It names the shared registry and cache, not
///   anything about the current build; dropping it would send the nested
///   build to a fresh default home and re-download the index.
/// * `RUSTC` is kept, because it does not start with `RUSTC_`. That is
///   intentional and not an oversight of the prefix rule: it names the
///   compiler binary to use, which the nested build must agree with.
///
/// Everything *not* matched here survives, which is the point of a
/// removal predicate rather than an allowlist: platform toolchain
/// variables (`SDKROOT` and `DEVELOPER_DIR` on macOS, `INCLUDE`, `LIB`,
/// and `LIBPATH` in an MSVC environment) are exactly the variables a
/// nested build cannot function without, and enumerating them correctly
/// for every Tier-1 platform is not verifiable from this project's single
/// development host.
///
/// A pure function over the variable name so the unit tests below can
/// drive every arm from string inputs. Driven only from a real process
/// environment, the `CARGO_HOME` arm would not execute wherever that
/// variable happens to be unset, leaving a D-014 region gap that depends
/// on the machine running the tests.
pub fn should_scrub_for_nested_cargo(name: &str) -> bool {
    if name == "CARGO_HOME" {
        return false;
    }
    name.starts_with("CARGO_")
        || name.starts_with("RUSTC_")
        || name == "RUSTFLAGS"
        || name == "RUSTDOCFLAGS"
        || name == "LLVM_PROFILE_FILE"
}

/// The variable a nested `cargo` invocation sets to announce itself, so a
/// build script re-entered from inside that nested build can detect the
/// recursion and do nothing instead of spawning another one.
pub const NESTED_BUILD_SENTINEL: &str = "PYCC_RT_NESTED_BUILD";

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_target_root() -> std::path::PathBuf {
        std::path::PathBuf::from("/workspace/target")
    }

    #[test]
    fn an_unset_cargo_target_dir_falls_back_to_the_manifest_relative_directory() {
        let manifest = std::path::Path::new("/workspace");
        assert_eq!(
            resolve_cargo_target_root(manifest, |_| None),
            workspace_target_root()
        );
    }

    #[test]
    fn an_empty_cargo_target_dir_is_treated_as_unset() {
        let manifest = std::path::Path::new("/workspace");
        assert_eq!(
            resolve_cargo_target_root(manifest, |_| Some(String::new())),
            workspace_target_root()
        );
    }

    #[test]
    fn an_absolute_cargo_target_dir_replaces_the_manifest_relative_directory() {
        let manifest = std::path::Path::new("/workspace");
        assert_eq!(
            resolve_cargo_target_root(manifest, |_| Some("/elsewhere/build".to_string())),
            std::path::PathBuf::from("/elsewhere/build")
        );
    }

    #[test]
    fn a_relative_cargo_target_dir_is_passed_through_unchanged() {
        // Cargo resolves a relative CARGO_TARGET_DIR against the invoking
        // process's working directory, so re-anchoring it on `manifest`
        // here would disagree with where Cargo actually wrote artifacts.
        let manifest = std::path::Path::new("/workspace");
        assert_eq!(
            resolve_cargo_target_root(manifest, |_| Some("build-out".to_string())),
            std::path::PathBuf::from("build-out")
        );
    }

    /// Answers only for `CARGO_BUILD_TARGET_DIR`, so a test can prove the
    /// second precedence level is reached when the first is unset. A
    /// non-capturing closure, so it still coerces to the `fn` pointer.
    fn only_build_target_dir(key: &str) -> Option<String> {
        match key {
            "CARGO_BUILD_TARGET_DIR" => Some("/from-build-config".to_string()),
            _ => None,
        }
    }

    #[test]
    fn cargo_build_target_dir_is_honored_when_cargo_target_dir_is_unset() {
        // Cargo's generic config-to-env mapping of `build.target-dir`.
        // Measured: `CARGO_BUILD_TARGET_DIR=<dir> cargo build` writes to
        // `<dir>/debug` and creates no local `target/`.
        let manifest = std::path::Path::new("/workspace");
        assert_eq!(
            resolve_cargo_target_root(manifest, only_build_target_dir),
            std::path::PathBuf::from("/from-build-config")
        );
    }

    /// Answers for both variables, so a test can prove the first
    /// precedence level wins. A named `fn` rather than an inline closure
    /// on purpose: the resolver short-circuits on `CARGO_TARGET_DIR` and
    /// never asks for the second key, so the other arms are only ever
    /// reached by calling this directly -- which the test below does,
    /// keeping every arm covered under D-014 rather than leaving two
    /// unexecuted regions inside an anonymous closure.
    fn both_target_dirs(key: &str) -> Option<String> {
        match key {
            "CARGO_TARGET_DIR" => Some("/from-env".to_string()),
            "CARGO_BUILD_TARGET_DIR" => Some("/from-build-config".to_string()),
            _ => None,
        }
    }

    #[test]
    fn cargo_target_dir_outranks_cargo_build_target_dir_when_both_are_set() {
        // The fixture genuinely answers for both variables -- asserted
        // directly, since the resolver never asks for the second one.
        assert_eq!(
            both_target_dirs("CARGO_BUILD_TARGET_DIR"),
            Some("/from-build-config".to_string())
        );
        assert_eq!(both_target_dirs("SOMETHING_ELSE"), None);
        // Measured: with both set, artifacts land under CARGO_TARGET_DIR
        // and the CARGO_BUILD_TARGET_DIR path is never created.
        let manifest = std::path::Path::new("/workspace");
        assert_eq!(
            resolve_cargo_target_root(manifest, both_target_dirs),
            std::path::PathBuf::from("/from-env")
        );
    }

    #[test]
    fn an_empty_cargo_build_target_dir_is_also_treated_as_unset() {
        let manifest = std::path::Path::new("/workspace");
        assert_eq!(
            resolve_cargo_target_root(manifest, |key| match key {
                "CARGO_BUILD_TARGET_DIR" => Some(String::new()),
                _ => None,
            }),
            workspace_target_root()
        );
    }

    #[test]
    fn the_env_lookup_reads_the_named_variable_from_the_process() {
        // Three assertions rather than one, because agreeing with
        // `std::env::var("CARGO_TARGET_DIR")` alone is vacuous wherever
        // the variable is unset (`None == None` holds even for a function
        // that always returns `None`), which is the common local case.
        // A name no environment defines must come back `None`...
        assert_eq!(
            cargo_target_dir_from_env("PYCC_ARTIFACT_LAYOUT_UNSET_PROBE"),
            None
        );
        // ...and the lookup must use the key it is handed rather than a
        // hardcoded one, which a second, unrelated key demonstrates
        // wherever the environment defines it (and states truthfully, not
        // flakily, where it does not).
        assert_eq!(
            cargo_target_dir_from_env("PATH"),
            std::env::var("PATH").ok()
        );
        assert_eq!(
            cargo_target_dir_from_env("CARGO_TARGET_DIR"),
            std::env::var("CARGO_TARGET_DIR").ok()
        );
    }

    #[test]
    fn finds_the_native_lib_dir_when_it_exists() {
        let root = workspace_target_root();
        let result = find_pycc_rt_lib_dir_in(&root, None, false, |_| true);
        assert_eq!(result, Ok(root.join("debug")));
    }

    #[test]
    fn reports_a_clean_error_when_the_native_build_is_missing() {
        let root = workspace_target_root();
        let err = find_pycc_rt_lib_dir_in(&root, None, false, |_| false).unwrap_err();
        assert!(err.contains("cargo build -p pycc_rt"));
        // Must not suggest --release for a debug-profile lookup.
        assert!(!err.contains("--release"));
        // The persistent-redirect recovery (#869) is named alongside the
        // rebuild command, and the message stays a single line.
        assert!(err.contains("with CARGO_TARGET_DIR set"));
        assert!(!err.contains('\n'));
    }

    #[test]
    fn finds_the_target_specific_lib_dir_when_it_exists() {
        let root = workspace_target_root();
        let result =
            find_pycc_rt_lib_dir_in(&root, Some("x86_64-unknown-linux-gnu"), false, |_| true);
        assert_eq!(
            result,
            Ok(root.join("x86_64-unknown-linux-gnu").join("debug"))
        );
    }

    #[test]
    fn reports_a_clean_error_naming_the_target_when_its_build_is_missing() {
        let root = workspace_target_root();
        let err =
            find_pycc_rt_lib_dir_in(&root, Some("x86_64-unknown-linux-gnu"), false, |_| false)
                .unwrap_err();
        assert!(err.contains("x86_64-unknown-linux-gnu"));
        assert!(err.contains("rustup target add"));
        assert!(err.contains("with CARGO_TARGET_DIR set"));
        assert!(!err.contains('\n'));
    }

    #[test]
    fn finds_the_native_release_lib_dir_when_it_exists() {
        let root = workspace_target_root();
        let result = find_pycc_rt_lib_dir_in(&root, None, true, |_| true);
        assert_eq!(result, Ok(root.join("release")));
    }

    #[test]
    fn reports_a_clean_error_naming_release_when_the_native_release_build_is_missing() {
        let root = workspace_target_root();
        let err = find_pycc_rt_lib_dir_in(&root, None, true, |_| false).unwrap_err();
        assert!(err.contains("cargo build --release -p pycc_rt"));
        // Not a literal separator-bearing substring check: `dir.display()`
        // renders with the platform's native separator, so a hardcoded
        // forward-slash needle would fail on Windows -- a real bug caught
        // by the pinned reviewer in an earlier round. Joining a `PathBuf`
        // compares components instead of literal separator bytes.
        let expected_dir = root.join("release");
        assert!(err.contains(&expected_dir.display().to_string()));
        assert!(err.contains("with CARGO_TARGET_DIR set"));
    }

    #[test]
    fn finds_the_target_specific_release_lib_dir_when_it_exists() {
        let root = workspace_target_root();
        let result =
            find_pycc_rt_lib_dir_in(&root, Some("x86_64-unknown-linux-gnu"), true, |_| true);
        assert_eq!(
            result,
            Ok(root.join("x86_64-unknown-linux-gnu").join("release"))
        );
    }

    #[test]
    fn reports_a_clean_error_naming_the_target_and_release_when_its_build_is_missing() {
        let root = workspace_target_root();
        let err = find_pycc_rt_lib_dir_in(&root, Some("x86_64-unknown-linux-gnu"), true, |_| false)
            .unwrap_err();
        assert!(err.contains("x86_64-unknown-linux-gnu"));
        assert!(err.contains("rustup target add"));
        assert!(err.contains("cargo build --release --target x86_64-unknown-linux-gnu -p pycc_rt"));
        assert!(err.contains("with CARGO_TARGET_DIR set"));
    }

    #[test]
    fn a_redirected_target_root_is_honored_end_to_end() {
        // The #629 behavior in one assertion: with CARGO_TARGET_DIR set,
        // the lookup lands under the redirected root, not under
        // `<manifest>/target`.
        let root = resolve_cargo_target_root(std::path::Path::new("/workspace"), |_| {
            Some("/elsewhere/build".to_string())
        });
        let result = find_pycc_rt_lib_dir_in(&root, None, false, |_| true);
        assert_eq!(
            result,
            Ok(std::path::PathBuf::from("/elsewhere/build").join("debug"))
        );
    }

    /// A path literal that is genuinely absolute on the host platform.
    ///
    /// A bare leading slash is *rooted* but not absolute on Windows --
    /// `Path::is_absolute` there additionally requires a drive or UNC
    /// prefix -- so a Unix-shaped literal would send
    /// `anchor_target_root_for_build_script` down its relative branch on
    /// exactly one Tier-1 platform, testing the opposite of what the test
    /// below names. A real resolved target root always carries whatever
    /// prefix its platform requires.
    #[cfg(windows)]
    const ABSOLUTE_ELSEWHERE: &str = r"C:\elsewhere\build";
    #[cfg(not(windows))]
    const ABSOLUTE_ELSEWHERE: &str = "/elsewhere/build";

    #[test]
    fn an_absolute_resolved_root_is_anchored_unchanged_and_never_diverges() {
        let anchored = anchor_target_root_for_build_script(
            std::path::Path::new(ABSOLUTE_ELSEWHERE),
            std::path::Path::new("/workspace"),
            std::path::Path::new("/somewhere/else/out"),
        );
        assert_eq!(anchored.root, std::path::PathBuf::from(ABSOLUTE_ELSEWHERE));
        assert!(!anchored.diverged);
    }

    #[test]
    fn a_relative_resolved_root_containing_out_dir_is_anchored_without_divergence() {
        let anchored = anchor_target_root_for_build_script(
            std::path::Path::new("build-out"),
            std::path::Path::new("/workspace"),
            std::path::Path::new("/workspace/build-out/debug/build/pycc_codegen-abc/out"),
        );
        assert_eq!(
            anchored.root,
            std::path::PathBuf::from("/workspace/build-out")
        );
        assert!(!anchored.diverged);
    }

    #[test]
    fn a_relative_resolved_root_not_containing_out_dir_is_reported_as_divergent() {
        // Cargo was invoked from somewhere other than the workspace root,
        // so it resolved the same relative value against a different
        // directory: the anchored guess names a place Cargo is not
        // reading from, and the caller must warn.
        let anchored = anchor_target_root_for_build_script(
            std::path::Path::new("build-out"),
            std::path::Path::new("/workspace"),
            std::path::Path::new("/elsewhere/build-out/debug/build/pycc_codegen-abc/out"),
        );
        assert_eq!(
            anchored.root,
            std::path::PathBuf::from("/workspace/build-out")
        );
        assert!(anchored.diverged);
    }

    #[test]
    fn the_nested_cargo_scrub_removes_the_outer_build_s_own_namespace() {
        for removed in [
            "CARGO_MANIFEST_DIR",
            "CARGO_TARGET_DIR",
            "CARGO_PKG_NAME",
            "CARGO_ENCODED_RUSTFLAGS",
            "RUSTC_WRAPPER",
            "RUSTC_WORKSPACE_WRAPPER",
            "RUSTC_BOOTSTRAP",
            "RUSTFLAGS",
            "RUSTDOCFLAGS",
            "LLVM_PROFILE_FILE",
        ] {
            assert!(should_scrub_for_nested_cargo(removed), "{removed}");
        }
    }

    #[test]
    fn the_nested_cargo_scrub_keeps_the_shared_cache_the_compiler_and_the_toolchain() {
        for kept in [
            "CARGO_HOME",
            // Bare `CARGO` is the path to the cargo binary itself, which
            // the build script re-invokes for the nested build. The
            // prefix rule spells `CARGO_` with the underscore precisely
            // so this one survives.
            "CARGO",
            "RUSTC",
            "RUSTUP_HOME",
            "RUSTUP_TOOLCHAIN",
            "PATH",
            "SDKROOT",
            "DEVELOPER_DIR",
            "INCLUDE",
            "LIB",
            "LIBPATH",
        ] {
            assert!(!should_scrub_for_nested_cargo(kept), "{kept}");
        }
    }

    #[test]
    fn the_recursion_sentinel_is_outside_the_scrubbed_namespace() {
        // The sentinel must survive into the nested build's own build
        // scripts; if it were scrubbed, the recursion guard could never
        // fire and a four-level nesting would be possible.
        assert!(!should_scrub_for_nested_cargo(NESTED_BUILD_SENTINEL));
    }

    #[test]
    fn an_msvc_target_uses_the_dot_lib_filename_regardless_of_host() {
        // The regression this guards: pycc_rt_lib_filename used to be a
        // host-#[cfg(windows)] constant, so requesting an -msvc target from
        // this (non-Windows) test runner silently checked for the wrong
        // filename (libpycc_rt.a instead of pycc_rt.lib).
        assert_eq!(
            pycc_rt_lib_filename(Some("x86_64-pc-windows-msvc")),
            "pycc_rt.lib"
        );
    }

    #[test]
    fn a_non_msvc_target_uses_the_lib_prefix_dot_a_filename() {
        assert_eq!(
            pycc_rt_lib_filename(Some("x86_64-unknown-linux-gnu")),
            "libpycc_rt.a"
        );
    }
}
