mod build_pipeline;
mod cli;
mod embed;
mod ext_build;
mod ext_output;
mod foreign_import;
mod frontend;
mod interop_policy;
mod lock;
mod memoryview_mode;
mod modules;
mod project_config;
mod source;

use build_pipeline::try_build;
use clap::Parser;
use cli::{Cli, Command, ErrorFormat, OutputFormat};
use embed::LibpythonLink;
use frontend::{check_frontend, report_check_failure};
use std::path::Path;
use std::process::ExitCode;

/// The five Tier-1 targets, in ARCHITECTURE.md's own "Cross-platform (hard
/// requirement)" table order (Linux, macOS, Windows; x86_64 before aarch64
/// within each OS pair). `pycc version --verbose` reports this list per
/// CLI_SPEC.md's command table; nothing else in the codebase enumerates
/// Tier-1 at runtime, so this constant is that table's only code mirror and
/// changes only when the table does. `pycc lock` also checks the host's
/// triple and every `pycc.lock` section's triple against it.
const TIER1_TARGETS: [&str; 5] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Build {
            path,
            out,
            target,
            release,
            ext,
            foreign_relative_imports,
            static_libpython,
            interop,
        } => {
            // Resolved here, not inside `try_build`: this consumption point
            // (a neighboring `pycc.toml`'s `[build] opt = "release"` as a
            // default profile) is scoped to `pycc build` specifically --
            // CLI_SPEC.md/ROADMAP.md both document it that way, and `run`
            // has no `--release` flag of its own to override it with if a
            // user doesn't want that default applied. Resolving it here,
            // before `try_build` ever runs, is what keeps `run`'s own
            // hardcoded `false` (below) actually final.
            let release = resolve_release_flag(release, &path);
            // Resolved here for the same reason (D-251): `run` never links
            // libpython statically, whatever the manifest says.
            let link = resolve_libpython_link(static_libpython, ext, &path);
            // Caller-owned scratch (#783): the temp object `try_build` emits
            // lives inside this `ScratchDir`, so every exit from this arm --
            // success and each error path alike -- removes it on drop. The
            // user's `-o` output is untouched (it is the explicitly
            // requested persistent output #779 carves out). Created before
            // any frontend work runs: an unusable system temp directory is
            // an environment failure that fails fast here (exit 2, like
            // `init`'s unreadable cwd), before the source is even read.
            let scratch = match create_scratch("build") {
                Ok(scratch) => scratch,
                Err(code) => return code,
            };
            // The toolchain is read from the environment here, in the
            // command arm, for the same reason `release` is resolved here:
            // `try_build` takes it by injection so a test can supply one
            // without setting a process-wide environment variable, which
            // would race every other test in this binary.
            let toolchain = ext.then(ext_build::ExtToolchain::from_env);
            match try_build(
                &path,
                &out,
                target.as_deref(),
                release,
                &scratch.join("main.o"),
                toolchain.as_ref(),
                &embed::EmbedToolchain::from_env().with_link(link),
                interop.into_cli(),
                foreign_relative_imports,
            ) {
                Ok(()) => ExitCode::SUCCESS,
                Err(code) => code,
            }
        }
        Command::Run {
            path,
            args,
            interop,
        } => run(&path, &args, interop.into_cli()),
        Command::Version { verbose } => {
            // `pycc {version}` comes from the manifest (`CARGO_PKG_VERSION`),
            // so it can't silently rot when the crate bumps. `rustc
            // {version}` used to come from `CARGO_PKG_RUST_VERSION`, but
            // that's the manifest's `rust-version` (MSRV) contract, not the
            // compiler that actually produced this binary -- a newer
            // installed rustc than the declared minimum builds cleanly and
            // silently makes the two diverge (#247). `build.rs` captures the
            // real build-time `rustc --version` into
            // `PYCC_BUILD_RUSTC_VERSION` instead. "LLVM 22.1.1" stays a
            // literal deliberately: it states D-015's pinned contract value,
            // and whether the *installed* LLVM actually matches that pin is
            // #75's open scope, not this line's job.
            println!(
                "pycc {} (rustc {}, LLVM 22.1.1)",
                env!("CARGO_PKG_VERSION"),
                env!("PYCC_BUILD_RUSTC_VERSION"),
            );
            if verbose {
                // CLI_SPEC.md's `pycc version --verbose` row promises the
                // target list; the set and order mirror the one authoritative
                // Tier-1 table in ARCHITECTURE.md ("Cross-platform (hard
                // requirement)"), which has no runtime representation
                // anywhere else in the codebase.
                println!("tier-1 targets:");
                for target in TIER1_TARGETS {
                    println!("  {target}");
                }
            }
            ExitCode::SUCCESS
        }
        Command::Check {
            paths,
            error_format,
            interop,
        } => check_paths(&paths, error_format, interop.into_cli()),
        Command::Lock {
            path,
            check,
            interop,
        } => {
            // Read from the environment here, like `build`'s toolchain: the
            // lock is taken from the interpreter an embedded build bundles.
            let toolchain = embed::EmbedToolchain::from_env();
            match lock::run_lock(&path, check, interop.into_cli(), &toolchain) {
                Ok(()) => ExitCode::SUCCESS,
                Err(failure) => ExitCode::from(lock::report_lock_failure(failure)),
            }
        }
        Command::Init { name } => {
            // `std::env::current_dir()` is fallible: the process's cwd may
            // have been deleted, unmounted, or become otherwise inaccessible
            // after launch (#251). This is an invocation/environment error
            // controlled outside pycc, not an internal invariant, so it is
            // reported as an exit-2 diagnostic (CLI_SPEC.md's exit-code
            // contract) rather than a panic. No scaffold write is attempted
            // and no fallback directory is used.
            match std::env::current_dir() {
                Ok(cwd) => match init(name.as_deref(), &cwd) {
                    Ok(()) => {
                        println!("Created pycc.toml and src/main.py");
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("error: pycc init failed: {e}");
                        ExitCode::from(2)
                    }
                },
                Err(e) => {
                    eprintln!("error: pycc init failed: cannot read current directory: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Command::Explain { code, format } => explain(&code, format),
        Command::Test | Command::Clean => {
            eprintln!("pycc: this subcommand is not yet implemented");
            ExitCode::from(2)
        }
    }
}

/// `pycc explain <code> [--format human|json]`: prints the code, severity,
/// a one-line summary, and a longer explanation with a worked example for
/// a recognized diagnostic code (D-150, `pycc_diag::explain`). Exits `0`
/// for a recognized code in either format. Exits `2` for an unrecognized
/// code, with a plain stderr message regardless of `--format` -- an
/// unrecognized code is an out-of-band invocation failure, not a
/// diagnostic occurrence, so it is never subject to `--format` the way
/// `check`'s own out-of-band `frontend::FrontendFailure::Input` class ("could not
/// read ...") is never subject to `--error-format` either.
fn explain(code: &str, format: OutputFormat) -> ExitCode {
    match pycc_diag::explain::find(code) {
        Some(entry) => {
            match format {
                OutputFormat::Human => {
                    print!("{}", pycc_diag::explain::render_explanation_human(entry));
                }
                OutputFormat::Json => {
                    println!("{}", pycc_diag::explain::render_explanation_json(entry));
                }
            }
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("error: unknown diagnostic code `{code}`");
            ExitCode::from(2)
        }
    }
}

/// `pycc init [NAME]`: testable core of the `Command::Init` arm. Takes
/// `dir` as a parameter instead of calling `std::env::current_dir()`
/// internally -- matching `pycc_codegen::artifact_layout`'s
/// `find_pycc_rt_lib_dir_in` dependency-injection split -- so a test can
/// exercise `project_config::scaffold`'s error path (an existing `pycc.toml`,
/// #237's refusal contract) without mutating this test process's real,
/// shared working directory.
fn init(name: Option<&str>, dir: &Path) -> Result<(), String> {
    project_config::scaffold(name, dir).map_err(|e| e.to_string())
}

/// `pycc check`: parse + HIR-lowering + type-checking only, no codegen --
/// CLI_SPEC.md's contract for this subcommand (ruff-fast, no codegen).
/// `error_format`: "human" (default) or "json", matching CLI_SPEC.md's
/// `--error-format` flag. Compile diagnostics are printed to stdout; input
/// errors are printed to stderr. Every supplied file is checked before the
/// highest-precedence exit code is returned.
fn check_paths(
    paths: &[std::path::PathBuf],
    error_format: ErrorFormat,
    interop: interop_policy::InteropCli,
) -> ExitCode {
    if paths.is_empty() {
        eprintln!("error: `pycc check` requires at least one Python file in v0.1");
        return ExitCode::from(2);
    }

    let mut exit_code = 0;
    for path in paths {
        if let Err(failure) = check_frontend(path, interop) {
            exit_code = exit_code.max(report_check_failure(failure, error_format));
        }
    }
    ExitCode::from(exit_code)
}

/// Creates the `pycc_scratch::ScratchDir` that holds a command's temporary
/// build artifacts (#783, Part 3 of #779), mapping a creation failure to
/// CLI_SPEC.md's exit-2 invocation/environment class -- an unusable system
/// temp directory is controlled outside pycc, exactly like the unstartable
/// linker driver below and `init`'s unreadable cwd, and is reported as an
/// actionable `error:` diagnostic rather than a panic. The returned handle
/// is caller-owned: `Drop` removes the directory tree (success, error, and
/// panic-unwind paths alike), so the caller must keep it alive until every
/// consumer of the files placed inside it -- the linker reading the temp
/// object, `run`'s child executing from inside it -- has finished.
///
/// Before creating the new root, this opportunistically sweeps
/// provably-stale pycc scratch roots left in the temp directory by dead
/// pycc processes (#784, Part 4 of #779) -- silent, bounded, and
/// best-effort, so the report is deliberately discarded and the sweep can
/// never change this command's output or exit code.
fn create_scratch(category: &str) -> Result<pycc_scratch::ScratchDir, ExitCode> {
    let _ = pycc_scratch::sweep_stale_roots();
    pycc_scratch::ScratchDir::new(category).map_err(|e| {
        eprintln!(
            "error: could not create a scratch directory under the system temp directory: {e}"
        );
        ExitCode::from(2)
    })
}

/// Resolves `pycc build`'s effective release/debug profile -- called only
/// from `main()`'s `Command::Build` arm, before `try_build` runs (`run` has
/// no `--release` flag and never calls this, so it's never subject to a
/// neighboring `pycc.toml`'s default; see `try_build`'s own doc comment).
/// An explicit `--release` (`explicit_release: true`) always wins outright
/// -- this is not project-mode directory resolution (`docs/CLI_SPEC.md`'s
/// deferred "PATH = ... project directory" case), just a narrow
/// default-profile consumption point (this PR's plan, Task 3 Step 6a,
/// moved here from Task 2's original Step 8 once this task's own `release`
/// field existed for it to combine with).
///
/// Otherwise, this looks for a `pycc.toml` next to `source_path` and uses
/// its `[build] opt == "release"` as the default. Every other outcome --
/// no `pycc.toml` there, one that fails to parse (`project_config::parse`
/// already rejects malformed TOML and any non-"3.14" `python`), or a
/// `[build] opt` value other than `"release"` (including no `[build]`
/// section at all, whose `opt` defaults to `None`) -- falls back to
/// `false`. A malformed neighboring `pycc.toml` must not abort an
/// otherwise-valid build over an optional default it doesn't even need.
fn resolve_release_flag(explicit_release: bool, source_path: &Path) -> bool {
    if explicit_release {
        return true;
    }
    neighboring_build_section(source_path)
        .is_some_and(|build| build.opt.as_deref() == Some("release"))
}

/// The `[build]` section of the `pycc.toml` next to `source_path`, or
/// `None` when there is no such file or it does not parse -- the shared
/// read behind `resolve_release_flag` and `resolve_libpython_link`, which
/// both treat a missing or malformed manifest as "no default".
fn neighboring_build_section(source_path: &Path) -> Option<project_config::BuildSection> {
    let dir = source_path.parent()?;
    let contents = std::fs::read_to_string(dir.join("pycc.toml")).ok()?;
    project_config::parse(&contents)
        .ok()
        .map(|config| config.build)
}

/// How `pycc build` links libpython into an embedded executable (D-251):
/// statically when `--static-libpython` is given or a neighboring
/// `pycc.toml` sets `[build] static = true`, by the same precedence and
/// the same forgiving manifest read as `resolve_release_flag`. An `--ext`
/// build loads into a host interpreter and links no libpython, so it is
/// always `Shared` there. The value matters only when the build embeds an
/// interpreter; a native, `--pure` or `--target` build ignores it.
fn resolve_libpython_link(explicit_static: bool, ext: bool, source_path: &Path) -> LibpythonLink {
    if ext {
        return LibpythonLink::Shared;
    }
    let from_manifest = || neighboring_build_section(source_path).and_then(|build| build.static_);
    if explicit_static || from_manifest() == Some(true) {
        LibpythonLink::Static
    } else {
        LibpythonLink::Shared
    }
}

/// Builds the `std::process::Command` that launches the freshly built
/// `binary`, forwarding `args` unchanged and in order as its process
/// arguments (CLI_SPEC.md's `pycc run [PATH] [-- args]` contract, #23).
/// `args` is `OsString`, not `String` (#824 item 2): a forwarded value can
/// be a non-UTF-8 byte sequence, and `Command::args` accepts `OsStr`
/// directly, so nothing here needs to re-validate or lossily convert it.
/// Factored out of `run` so the forwarding itself -- not just the full
/// build-and-execute pipeline -- has a direct test against a real child
/// process.
fn run_command(binary: &std::path::Path, args: &[std::ffi::OsString]) -> std::process::Command {
    let mut command = std::process::Command::new(binary);
    command.args(args);
    command
}

/// `pycc run` has no `--release` flag (undocumented in CLI_SPEC.md) and
/// always builds in the debug profile: the hardcoded `false` below reaches
/// `try_build` directly, which -- unlike `main()`'s `Command::Build` arm --
/// never consults a neighboring `pycc.toml`'s `[build] opt = "release"`
/// default, so this stays unconditional regardless of what any nearby
/// `pycc.toml` names.
///
/// Both temporary artifacts -- the codegen object and the linked
/// executable -- live inside one caller-owned `create_scratch` directory
/// (#783). The `ScratchDir`'s own `pid`/`nanos`/`seq` name already carries
/// all the uniqueness the old `pycc_run_{pid}` file name provided, so plain
/// `out`/`main.o` file names inside it are enough. Binding order is
/// load-bearing: `scratch` stays alive across `try_build` (the linker reads
/// the object and writes `out`) *and* across the call to
/// `run_built_binary` below, which itself calls `run_command(..).status()`
/// (the child executes from inside the scratch directory; `status()` waits
/// for it to exit), so `Drop` removes the directory only after the child
/// has terminated -- do not restructure this to return before that wait.
fn run(path: &Path, args: &[std::ffi::OsString], interop: interop_policy::InteropCli) -> ExitCode {
    let scratch = match create_scratch("run") {
        Ok(scratch) => scratch,
        Err(code) => return code,
    };
    // An embedded program's sidecar lands at `scratch/out.pycc` and goes
    // with the scratch directory (Part 1 of #1028).
    let out = scratch.join("out");
    let embed = embed::EmbedToolchain::from_env();
    if let Err(code) = try_build(
        path,
        &out,
        None,
        false,
        &scratch.join("main.o"),
        None,
        &embed,
        interop,
        false,
    ) {
        return code;
    }
    ExitCode::from(run_built_binary(&out, args))
}

/// Spawns the just-linked binary at `out` and waits for it, mapping the
/// result to `pycc run`'s exit code. Factored out of `run` (#249) so the
/// exec-failure path -- `Command::status()` failing to even start the
/// child, e.g. a permissions error or the binary having vanished between
/// link and spawn -- has a direct seam to test: calling this function with
/// a path that cannot be executed reaches the `Err` arm deterministically,
/// without needing `run`'s own scratch-directory and build machinery to
/// manufacture that failure. Returns a plain `u8` rather than `ExitCode`
/// (mirroring `frontend::report_check_failure`/`report_build_failure`) so a
/// test can assert the exact numeric value instead of only "did not
/// panic" -- `ExitCode` itself has no `PartialEq` (see
/// `create_scratch_tests`'s own note on this).
fn run_built_binary(out: &Path, args: &[std::ffi::OsString]) -> u8 {
    let status = match run_command(out, args).status() {
        Ok(status) => status,
        // Failing to *start* the built binary (permission denied, the file
        // vanishing between link and spawn) is an ordinary environment
        // failure, not a pycc invariant -- report it like the linker's own
        // spawn failure in `try_build` above (CLI_SPEC.md's exit-2
        // invocation/environment class) instead of panicking with a raw
        // backtrace.
        Err(e) => {
            eprintln!(
                "error: could not run the built program `{}`: {e}",
                pycc_diag::display_path(&out.to_string_lossy())
            );
            return 2;
        }
    };
    // Any unsuccessful termination maps to CLI_SPEC.md's stable 101 on
    // every platform, including Unix signal termination where `code()` is
    // `None` and Windows abort statuses that do not fit in a u8. A native
    // program has no user-controlled non-zero exit status, so there this is
    // a runtime panic, trap, or uncaught failure. An embedded program's
    // `sys.exit(n)` is user-controlled (Part 1 of #1028), and `pycc run`
    // still maps it to 101: preserving `n` is a CLI contract change left out
    // of that part (docs/CLI_SPEC.md "Exit codes").
    if status.success() { 0 } else { 101 }
}

#[cfg(test)]
mod init_tests {
    use super::*;
    use pycc_scratch::ScratchDir;

    #[test]
    fn succeeds_and_scaffolds_a_project_in_a_writable_directory() {
        let dir = ScratchDir::new("main_init_test").expect("failed to create scratch dir");

        assert_eq!(init(Some("initdirect"), &dir), Ok(()));
        assert!(dir.join("pycc.toml").exists());
        assert!(dir.join("src").join("main.py").exists());
    }

    #[test]
    fn reports_the_scaffold_error_when_pycc_toml_already_exists() {
        // #237: an existing `pycc.toml` is the deterministic cross-platform
        // error case for `init`'s error arm now that a nonexistent target
        // directory scaffolds successfully (the inverted write order's
        // `create_dir_all` creates it -- see `project_config.rs`'s
        // `scaffold_creates_a_missing_target_directory`). The refusal
        // message flows through `init`'s io::Error -> String mapping.
        let dir = ScratchDir::new("main_init_existing_toml").expect("failed to create scratch dir");
        std::fs::write(dir.join("pycc.toml"), "user content").unwrap();

        let err = init(Some("irrelevant"), &dir).unwrap_err();
        assert!(err.contains("`pycc.toml` already exists"));
        assert_eq!(
            std::fs::read_to_string(dir.join("pycc.toml")).unwrap(),
            "user content"
        );
    }
}

#[cfg(test)]
mod release_flag_tests {
    use super::*;
    use pycc_scratch::ScratchDir;

    fn scratch_dir(label: &str) -> ScratchDir {
        ScratchDir::new(&format!("main_release_flag_{label}"))
            .expect("failed to create scratch dir")
    }

    #[test]
    fn an_explicit_release_flag_always_wins() {
        // True regardless of the source path or any neighboring file --
        // an explicit `--release` never needs a `pycc.toml` to exist at all.
        assert!(resolve_release_flag(true, Path::new("/does/not/exist.py")));
    }

    #[test]
    fn a_source_path_with_no_parent_falls_back_to_false() {
        // `Path::new("").parent()` is documented to return `None` on every
        // platform (an empty path has no components at all) -- the one
        // input that reliably exercises this function's `None` branch
        // without relying on any real filesystem root.
        assert!(!resolve_release_flag(false, Path::new("")));
    }

    #[test]
    fn no_neighboring_pycc_toml_falls_back_to_false() {
        let dir = scratch_dir("no_toml");
        let source_path = dir.join("main.py");

        assert!(!resolve_release_flag(false, &source_path));
    }

    #[test]
    fn a_malformed_neighboring_pycc_toml_falls_back_to_false_instead_of_aborting() {
        // A build must not fail merely because an optional default it
        // doesn't even need happens to be unreadable.
        let dir = scratch_dir("malformed_toml");
        std::fs::write(dir.join("pycc.toml"), "this is not [valid toml").unwrap();
        let source_path = dir.join("main.py");

        assert!(!resolve_release_flag(false, &source_path));
    }

    #[test]
    fn a_neighboring_pycc_toml_with_a_non_release_opt_falls_back_to_false() {
        let dir = scratch_dir("debug_opt");
        std::fs::write(
            dir.join("pycc.toml"),
            "[project]\nname = \"t\"\nentry = \"main.py\"\npython = \"3.14\"\n\n\
             [build]\nopt = \"debug\"\n",
        )
        .unwrap();
        let source_path = dir.join("main.py");

        assert!(!resolve_release_flag(false, &source_path));
    }

    #[test]
    fn a_neighboring_pycc_toml_with_no_build_section_falls_back_to_false() {
        let dir = scratch_dir("no_build_section");
        std::fs::write(
            dir.join("pycc.toml"),
            "[project]\nname = \"t\"\nentry = \"main.py\"\npython = \"3.14\"\n",
        )
        .unwrap();
        let source_path = dir.join("main.py");

        assert!(!resolve_release_flag(false, &source_path));
    }

    #[test]
    fn a_neighboring_pycc_toml_with_a_release_opt_becomes_the_default() {
        let dir = scratch_dir("release_opt");
        std::fs::write(
            dir.join("pycc.toml"),
            "[project]\nname = \"t\"\nentry = \"main.py\"\npython = \"3.14\"\n\n\
             [build]\nopt = \"release\"\n",
        )
        .unwrap();
        let source_path = dir.join("main.py");

        assert!(resolve_release_flag(false, &source_path));
    }

    fn manifest_with_static(dir: &Path, value: &str) {
        std::fs::write(
            dir.join("pycc.toml"),
            format!(
                "[project]\nname = \"t\"\nentry = \"main.py\"\npython = \"3.14\"\n\n\
                 [build]\nstatic = {value}\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn the_static_libpython_flag_links_statically_without_a_manifest() {
        let link = resolve_libpython_link(true, false, Path::new("/does/not/exist.py"));
        assert_eq!(link, LibpythonLink::Static);
    }

    #[test]
    fn a_manifest_static_key_is_the_default_and_false_or_absent_is_shared() {
        let dir = scratch_dir("static_key");
        let source_path = dir.join("main.py");
        assert_eq!(
            resolve_libpython_link(false, false, &source_path),
            LibpythonLink::Shared
        );
        manifest_with_static(&dir, "true");
        assert_eq!(
            resolve_libpython_link(false, false, &source_path),
            LibpythonLink::Static
        );
        manifest_with_static(&dir, "false");
        assert_eq!(
            resolve_libpython_link(false, false, &source_path),
            LibpythonLink::Shared
        );
        // The flag wins over a manifest that says `false`.
        assert_eq!(
            resolve_libpython_link(true, false, &source_path),
            LibpythonLink::Static
        );
    }

    #[test]
    fn an_ext_build_never_links_libpython_statically() {
        let dir = scratch_dir("static_ext");
        manifest_with_static(&dir, "true");
        let source_path = dir.join("main.py");
        assert_eq!(
            resolve_libpython_link(false, true, &source_path),
            LibpythonLink::Shared
        );
    }
}

#[cfg(test)]
mod create_scratch_tests {
    use super::*;

    #[test]
    fn returns_a_live_directory_that_drop_removes() {
        let scratch =
            create_scratch("main_create_scratch_ok").expect("create_scratch should succeed");
        let path = scratch.to_path_buf();
        assert!(
            path.is_dir(),
            "create_scratch must hand back an already-created directory"
        );
        drop(scratch);
        assert!(
            !path.exists(),
            "dropping the handle must remove the scratch directory"
        );
    }

    #[test]
    fn maps_a_creation_failure_to_an_error() {
        // A NUL byte is invalid in a path component on every platform this
        // repo targets, so `create_dir` fails portably -- mirroring
        // `pycc_scratch`'s own
        // `a_category_that_produces_an_invalid_path_component_propagates_the_create_dir_error`.
        // This exercises the `map_err` closure (its own coverage region)
        // inside this single `--cfg test` instantiation; `ExitCode` has no
        // `PartialEq`, so the *value* 2 is asserted at the e2e level
        // (tests/slice0.rs's bad-temp-dir tests), not here.
        let result = create_scratch("bad\0category");
        assert!(
            result.is_err(),
            "a NUL byte in the category should make create_scratch fail"
        );
    }
}

#[cfg(all(test, unix))]
mod run_command_tests {
    use super::run_command;
    use std::ffi::OsString;

    /// Proves `run_command` actually forwards its `args` unchanged and in
    /// order to the child process (#23), not merely that `pycc run` parses
    /// them: the Python language surface has no `sys.argv` yet, so a
    /// compiled program can't itself observe its own arguments, and this is
    /// the only way to verify forwarding against a real process rather than
    /// only the CLI parser. `/bin/echo` is POSIX-guaranteed, hence
    /// `#[cfg(unix)]`; the multi-value, dash-prefixed, and Unicode cases are
    /// already covered without a real process at the parser level in
    /// `cli.rs`, so this single case only needs to prove forwarding, not
    /// re-cover every value shape.
    #[test]
    fn forwards_args_unchanged_and_in_order_to_the_child_process() {
        let args: Vec<OsString> = vec!["first".into(), "-x".into(), "héllo".into()];

        let output = run_command(std::path::Path::new("/bin/echo"), &args)
            .output()
            .expect("/bin/echo should run");

        assert!(output.status.success());
        assert_eq!(output.stdout, b"first -x h\xc3\xa9llo\n");
    }

    #[test]
    fn forwards_no_args_when_the_slice_is_empty() {
        let output = run_command(std::path::Path::new("/bin/echo"), &[])
            .output()
            .expect("/bin/echo should run");

        assert!(output.status.success());
        assert_eq!(output.stdout, b"\n");
    }

    /// #824 item 2: a forwarded value that is not valid UTF-8 must reach
    /// the child process as the same opaque bytes, not get lossily
    /// replaced or rejected. `/bin/echo`'s own stdout is a byte stream, so
    /// this proves the forwarding survives all the way through a real
    /// child process, matching `cli.rs`'s parser-level
    /// `run_captures_non_utf8_trailing_args_as_opaque_bytes` proving the
    /// parse side alone.
    #[test]
    fn forwards_non_utf8_args_as_opaque_bytes_to_the_child_process() {
        use std::os::unix::ffi::OsStringExt;

        let args: Vec<OsString> = vec![OsString::from_vec(b"arg_\xff".to_vec())];

        let output = run_command(std::path::Path::new("/bin/echo"), &args)
            .output()
            .expect("/bin/echo should run");

        assert!(output.status.success());
        assert_eq!(output.stdout, b"arg_\xff\n");
    }
}

#[cfg(all(test, unix))]
mod run_built_binary_tests {
    use super::run_built_binary;

    /// #249: `run_built_binary` (factored out of `run`) used to
    /// `.status().expect("built binary should run")`, so a spawn failure --
    /// the built binary missing or unexecutable between link and spawn --
    /// panicked with a raw backtrace instead of reporting CLI_SPEC.md's
    /// exit-2 invocation/environment class, the same class
    /// `try_build`'s own linker-spawn-failure arm already uses. A
    /// nonexistent path makes `Command::status()` fail with `ENOENT`
    /// portably, reaching the `Err` arm directly -- no scratch directory or
    /// real build needed to manufacture the failure. `run_built_binary`
    /// returns a plain `u8` (not `ExitCode`, which has no `PartialEq` --
    /// see `create_scratch_tests`'s own note on this) specifically so this
    /// exact exit-2 value can be asserted directly.
    #[test]
    fn a_binary_that_cannot_be_spawned_reports_exit_code_2() {
        let out = std::path::Path::new("/nonexistent/pycc_249_test_binary");
        assert_eq!(run_built_binary(out, &[]), 2);
    }

    /// A binary that spawns and exits `0` reports exit code `0`. Exercises
    /// the `Ok` arm's success branch, which the spawn-failure test above
    /// never reaches.
    #[test]
    fn a_successfully_run_binary_reports_exit_code_0() {
        let out = std::path::Path::new("/usr/bin/true");
        assert_eq!(run_built_binary(out, &[]), 0);
    }

    /// A binary that spawns but exits non-zero reports the stable `101`
    /// (CLI_SPEC.md's "compiled program panicked/uncaught exception" exit
    /// code, reused here for any unsuccessful child termination). Exercises
    /// the `Ok` arm's failure branch.
    #[test]
    fn a_binary_that_exits_non_zero_reports_exit_code_101() {
        let out = std::path::Path::new("/usr/bin/false");
        assert_eq!(run_built_binary(out, &[]), 101);
    }
}
