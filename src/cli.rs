use crate::interop_policy::{InteropCli, InteropPolicy};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ErrorFormat {
    Human,
    Json,
}

/// `pycc explain <code>`'s own output-format flag. Deliberately not
/// `ErrorFormat`/`--error-format`: `explain`'s output documents a code, it
/// never reports an occurred error, so reusing `check`'s error-flavored
/// name and type would be actively misleading about what the command does
/// (see D-150).
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

/// D-128's interop flags (#1224), shared by `build`, `run` and `check`.
/// `docs/CLI_SPEC.md`'s `pycc.toml` `[interop]` section is the canonical
/// statement of how they combine with the manifest.
#[derive(Args, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InteropFlags {
    /// The interop policy for CPython-backed imports, overriding
    /// `pycc.toml`'s `[interop] policy`: `auto` admits every root,
    /// `allowlist` only the roots `[interop] allow` lists, `deny` none. An
    /// admitted root must still be one the build can embed (`I0403`).
    #[arg(long, value_enum)]
    pub interop_policy: Option<InteropPolicy>,
    /// Shorthand for `--interop-policy deny`; rejected together with any
    /// explicit `--interop-policy`, in either order.
    #[arg(long, conflicts_with = "interop_policy")]
    pub pure: bool,
}

impl InteropFlags {
    pub fn into_cli(self) -> InteropCli {
        InteropCli {
            policy: self.interop_policy,
            pure: self.pure,
        }
    }
}

#[derive(Parser)]
#[command(name = "pycc")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    Build {
        path: PathBuf,
        #[arg(short = 'o')]
        out: PathBuf,
        /// Cross-compile for a different Tier-1 target triple (e.g.
        /// x86_64-apple-darwin). Omit to build for the host's own default
        /// target -- the common case.
        #[arg(long)]
        target: Option<String>,
        /// Enable LLVM optimization (O3-equivalent whole-module pipeline).
        /// True cross-file LTO has no effect yet -- pycc compiles exactly
        /// one module per invocation until v0.4's multi-file support lands
        /// (D-094). Omit to use `pycc.toml`'s neighboring `[build] opt =
        /// "release"` default when one is present, or plain debug output
        /// otherwise.
        #[arg(long)]
        release: bool,
        /// Build a hosted CPython extension module (D-244) instead of a
        /// native executable: every public module-level function, every
        /// public `@staticmethod` and `@classmethod` of a public class, and
        /// every public instance method of a public class that some
        /// *published, constructible* class's method resolution order
        /// reaches, becomes
        /// a callable of one stable-ABI artifact that a CPython interpreter
        /// imports -- a method as `mod.Class.method`, an instance method on
        /// an instance the host builds with `mod.Class(...)`. Each class's
        /// method table is resolved through the class's namespace, so an
        /// inherited method is reachable on the derived class while any
        /// binding the derived class makes for that name -- another method,
        /// a `@property`, an `@abstractmethod` -- shadows it, publishing the
        /// derived binding or nothing. `-o` names
        /// that artifact; `docs/CLI_SPEC.md:23-63` is
        /// the canonical statement of how its suffix and module name are
        /// derived. `docs/RUNTIME.md`'s `ext` boundary section is the
        /// canonical admissibility matrix for what the boundary carries;
        /// any signature outside it is a `C0003` capability gap.
        ///
        /// The interop policy governs only the embedded mode, so `--ext`
        /// is rejected together with `--interop-policy` or `--pure` (D-244
        /// rule 3).
        #[arg(long, conflicts_with_all = ["interop_policy", "pure"])]
        ext: bool,
        /// Bind the entry module's relative `from .x import a` statements
        /// as CPython objects of the package the `--ext` artifact is
        /// imported under, resolved when it is imported, instead of as
        /// project modules (D-222's opt-in, #1366). Applies to the entry
        /// module's relative from-imports, at top level or nested in a
        /// module-level `if`/`try` block (#1383), and since #1382 to its
        /// absolute imports rooted at its own top-level package (the
        /// outermost directory above it that holds an `__init__.py`), in
        /// every `import`/`from` shape; a dependency's imports stay project
        /// imports (docs/CLI_SPEC.md). Requires `--ext`.
        #[arg(long, requires = "ext")]
        foreign_relative_imports: bool,
        /// Link libpython into the executable from the embed interpreter's
        /// static archive instead of bundling its shared library, and
        /// export its C-API symbols so a standard-library extension loads
        /// against them (D-251). Needs a CPython 3.14 whose `LIBPL` holds
        /// `libpython3.14.a`; a missing, thin or non-archive file is
        /// refused. A `pycc.lock` section is consumed as for a shared
        /// build, checked against the interpreter's shared library, or its
        /// archive when configured without one (#1272). Omit to use a neighboring
        /// `pycc.toml`'s `[build]
        /// static = true` when one is present, or the shared library
        /// otherwise. Only an embedded build uses it: a native, `--pure`
        /// or `--target` build ignores it, and `--ext` rejects it.
        #[arg(long, conflicts_with = "ext")]
        static_libpython: bool,
        #[command(flatten)]
        interop: InteropFlags,
    },
    Run {
        path: PathBuf,
        /// Every value after `--`, forwarded unchanged and in order as the
        /// generated program's process arguments (CLI_SPEC.md's
        /// `pycc run [PATH] [-- args]` contract, #23). `trailing_var_arg` +
        /// `allow_hyphen_values` so a value that itself looks like a flag
        /// (e.g. `-x`) is captured here instead of being rejected as an
        /// unrecognized `pycc` option.
        ///
        /// `OsString`, not `String` (#824 item 2): unlike a `String`
        /// positional, clap does not UTF-8-validate an `OsString` one, so a
        /// non-UTF-8 forwarded value (e.g. a byte sequence a shell can
        /// still pass through `$@`) is captured as an opaque byte sequence
        /// and forwarded to the child process unchanged, the same
        /// preserve-native-bytes treatment `PATH`/`OUT` already get via
        /// `PathBuf` (#249), instead of being rejected with a CLI parse
        /// error. `std::process::Command::args` accepts `OsStr`/`OsString`
        /// directly, so nothing downstream needs to re-decode these.
        ///
        /// #824 item 1 asked whether `trailing_var_arg` should require an
        /// explicit `--` before capturing anything. It still does not, but
        /// since `run` gained its own flags (#1224) the capture has a
        /// window: pycc flags (`--pure`, `--interop-policy`) are recognized
        /// before `PATH` and between `PATH` and the first forwarded value;
        /// the first forwarded value and everything after it are forwarded,
        /// and `--` forwards a value that shares a pycc flag's name
        /// (`pycc run app.py -- --pure`). A misspelled pycc flag after
        /// `PATH` is therefore forwarded like any other hyphen value, and so
        /// is every flag after it. `docs/CLI_SPEC.md`'s `run` contract is
        /// the canonical statement.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
        #[command(flatten)]
        interop: InteropFlags,
    },
    Check {
        paths: Vec<PathBuf>,
        /// CLI_SPEC.md's diagnostic-output contract: "human" (default) or "json".
        #[arg(long, value_enum, default_value = "human")]
        error_format: ErrorFormat,
        #[command(flatten)]
        interop: InteropFlags,
    },
    /// `pycc lock PATH [--check]`: records the CPython dependency closure
    /// PATH's embedded build will carry into `pycc.lock` (the pycc.lock
    /// decision entry, D-249; `docs/CLI_SPEC.md`'s `pycc.lock` section).
    Lock {
        path: PathBuf,
        /// Exit 0 only if `pycc.lock` already holds what `pycc lock` would
        /// write; exit 1 otherwise. Writes nothing.
        #[arg(long)]
        check: bool,
        #[command(flatten)]
        interop: InteropFlags,
    },
    Test,
    Explain {
        code: String,
        /// `pycc explain`'s own output-format flag (see `OutputFormat`'s
        /// doc comment for why this isn't `--error-format`).
        #[arg(long, value_enum, default_value = "human")]
        format: OutputFormat,
    },
    Init {
        name: Option<String>,
    },
    Clean,
    Version {
        #[arg(long)]
        verbose: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command, ErrorFormat, InteropFlags, InteropPolicy, OsString, OutputFormat};
    use clap::Parser;

    #[cfg(unix)]
    fn parsed_check_paths(command: Command) -> Option<Vec<std::path::PathBuf>> {
        match command {
            Command::Check { paths, .. } => Some(paths),
            _ => None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn check_paths_preserve_non_utf8_bytes() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let path = std::ffi::OsString::from_vec(b"staged_\xff.py".to_vec());
        let cli = Cli::try_parse_from([
            std::ffi::OsString::from("pycc"),
            std::ffi::OsString::from("check"),
            std::ffi::OsString::from("--"),
            path,
        ])
        .unwrap();
        let paths = parsed_check_paths(cli.command).unwrap();

        assert_eq!(paths[0].as_os_str().as_bytes(), b"staged_\xff.py");
        assert!(parsed_check_paths(Command::Clean).is_none());
    }

    #[cfg(unix)]
    fn parsed_build_path_and_out(
        command: Command,
    ) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        match command {
            Command::Build { path, out, .. } => Some((path, out)),
            _ => None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn build_path_and_out_preserve_non_utf8_bytes() {
        // Extends `check_paths_preserve_non_utf8_bytes`'s invariant to
        // `build` (#249): both the input path and the `-o` output path are
        // native filesystem paths, not text, so neither may be forced
        // through UTF-8 (CLI_SPEC.md's path contract).
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let path = std::ffi::OsString::from_vec(b"staged_\xff.py".to_vec());
        let out = std::ffi::OsString::from_vec(b"out_\xff".to_vec());
        let cli = Cli::try_parse_from([
            std::ffi::OsString::from("pycc"),
            std::ffi::OsString::from("build"),
            path,
            std::ffi::OsString::from("-o"),
            out,
        ])
        .unwrap();
        let (path, out) = parsed_build_path_and_out(cli.command).unwrap();

        assert_eq!(path.as_os_str().as_bytes(), b"staged_\xff.py");
        assert_eq!(out.as_os_str().as_bytes(), b"out_\xff");
        assert!(parsed_build_path_and_out(Command::Clean).is_none());
    }

    #[cfg(unix)]
    fn parsed_run_path(command: Command) -> Option<std::path::PathBuf> {
        match command {
            Command::Run { path, .. } => Some(path),
            _ => None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn run_path_preserves_non_utf8_bytes() {
        // Extends the same invariant to `run` (#249).
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let path = std::ffi::OsString::from_vec(b"staged_\xff.py".to_vec());
        let cli = Cli::try_parse_from([
            std::ffi::OsString::from("pycc"),
            std::ffi::OsString::from("run"),
            path,
        ])
        .unwrap();
        let path = parsed_run_path(cli.command).unwrap();

        assert_eq!(path.as_os_str().as_bytes(), b"staged_\xff.py");
        assert!(parsed_run_path(Command::Clean).is_none());
    }

    #[test]
    fn check_accepts_json_error_format_with_multiple_paths() {
        assert!(matches!(
            Cli::try_parse_from([
                "pycc",
                "check",
                "--error-format",
                "json",
                "first.py",
                "second.py",
            ])
            .unwrap()
            .command,
            Command::Check {
                error_format: ErrorFormat::Json,
                paths,
                interop,
            } if paths.len() == 2 && interop == InteropFlags::default()
        ));
    }

    fn parsed_explain(command: Command) -> Option<(String, OutputFormat)> {
        match command {
            Command::Explain { code, format } => Some((code, format)),
            _ => None,
        }
    }

    #[test]
    fn explain_accepts_json_format() {
        let cli = Cli::try_parse_from(["pycc", "explain", "T0001", "--format", "json"]).unwrap();
        let (code, format) = parsed_explain(cli.command).unwrap();
        assert_eq!(code, "T0001");
        assert_eq!(format, OutputFormat::Json);
        assert!(parsed_explain(Command::Clean).is_none());
    }

    #[test]
    fn explain_defaults_to_human_format() {
        let cli = Cli::try_parse_from(["pycc", "explain", "T0001"]).unwrap();
        let (_, format) = parsed_explain(cli.command).unwrap();
        assert_eq!(format, OutputFormat::Human);
    }

    fn parsed_build_release(command: Command) -> Option<bool> {
        match command {
            Command::Build { release, .. } => Some(release),
            _ => None,
        }
    }

    #[test]
    fn build_defaults_to_release_false_when_the_flag_is_omitted() {
        let cli = Cli::try_parse_from(["pycc", "build", "in.py", "-o", "out"]).unwrap();
        assert_eq!(parsed_build_release(cli.command), Some(false));
        assert!(parsed_build_release(Command::Clean).is_none());
    }

    #[test]
    fn build_accepts_an_explicit_release_flag() {
        let cli =
            Cli::try_parse_from(["pycc", "build", "in.py", "-o", "out", "--release"]).unwrap();
        assert_eq!(parsed_build_release(cli.command), Some(true));
    }

    fn parsed_run(command: Command) -> Option<(std::path::PathBuf, Vec<OsString>)> {
        match command {
            Command::Run { path, args, .. } => Some((path, args)),
            _ => None,
        }
    }

    #[test]
    fn run_with_no_trailing_args_captures_an_empty_vec() {
        // Preserves `pycc run app.py` behavior with no arguments (#23).
        let cli = Cli::try_parse_from(["pycc", "run", "app.py"]).unwrap();
        let (path, args) = parsed_run(cli.command).unwrap();
        assert_eq!(path, std::path::PathBuf::from("app.py"));
        assert!(args.is_empty());
        assert!(parsed_run(Command::Clean).is_none());
    }

    #[test]
    fn run_captures_multiple_trailing_args_in_order() {
        let cli = Cli::try_parse_from(["pycc", "run", "app.py", "--", "first", "second", "third"])
            .unwrap();
        let (path, args) = parsed_run(cli.command).unwrap();
        assert_eq!(path, std::path::PathBuf::from("app.py"));
        assert_eq!(
            args,
            vec![
                OsString::from("first"),
                OsString::from("second"),
                OsString::from("third"),
            ]
        );
    }

    #[test]
    fn run_captures_unicode_trailing_args_unchanged() {
        let cli =
            Cli::try_parse_from(["pycc", "run", "app.py", "--", "héllo", "世界", "🦀"]).unwrap();
        let (_, args) = parsed_run(cli.command).unwrap();
        assert_eq!(
            args,
            vec![
                OsString::from("héllo"),
                OsString::from("世界"),
                OsString::from("🦀"),
            ]
        );
    }

    #[test]
    fn run_captures_dash_prefixed_trailing_args_without_treating_them_as_pycc_options() {
        // A value starting with `-` after `--` (e.g. `-x`, `--flag`) must be
        // captured as a program argument, not rejected as an unrecognized
        // `pycc` option (the exact regression #23 reports: `pycc run app.py
        // -- hello` previously failed with "unexpected argument 'hello'
        // found").
        let cli =
            Cli::try_parse_from(["pycc", "run", "app.py", "--", "-x", "--flag", "hello"]).unwrap();
        let (path, args) = parsed_run(cli.command).unwrap();
        assert_eq!(path, std::path::PathBuf::from("app.py"));
        assert_eq!(
            args,
            vec![
                OsString::from("-x"),
                OsString::from("--flag"),
                OsString::from("hello"),
            ]
        );
    }

    #[test]
    fn run_captures_a_trailing_arg_without_requiring_an_explicit_separator() {
        // CLI_SPEC.md's #824 note: `trailing_var_arg` captures a value that
        // follows `path` whether or not `--` precedes it -- `--` documents
        // the always-safe form, and is required only to forward a value
        // that shares a pycc flag's name (#1224).
        let cli = Cli::try_parse_from(["pycc", "run", "app.py", "extra"]).unwrap();
        let (path, args) = parsed_run(cli.command).unwrap();
        assert_eq!(path, std::path::PathBuf::from("app.py"));
        assert_eq!(args, vec![OsString::from("extra")]);
    }

    #[cfg(unix)]
    #[test]
    fn run_captures_non_utf8_trailing_args_as_opaque_bytes() {
        // #824 item 2: a forwarded value after `--` used to be parsed as
        // `String`, so a non-UTF-8 byte sequence (still a value a shell can
        // pass through `$@`) was rejected with a CLI parse error instead of
        // being forwarded like `PATH`/`OUT` already are (#249). `OsString`
        // accepts it losslessly.
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let bad_arg = OsString::from_vec(b"arg_\xff".to_vec());
        let cli = Cli::try_parse_from([
            OsString::from("pycc"),
            OsString::from("run"),
            OsString::from("app.py"),
            OsString::from("--"),
            bad_arg.clone(),
        ])
        .unwrap();
        let (_, args) = parsed_run(cli.command).unwrap();

        assert_eq!(args.len(), 1);
        assert_eq!(args[0].as_bytes(), bad_arg.as_bytes());
    }

    fn parsed_run_interop(command: Command) -> Option<(InteropFlags, Vec<OsString>)> {
        match command {
            Command::Run { interop, args, .. } => Some((interop, args)),
            _ => None,
        }
    }

    fn run_interop(argv: &[&str]) -> (InteropFlags, Vec<OsString>) {
        let cli = Cli::try_parse_from(argv).expect("the command line parses");
        parsed_run_interop(cli.command).expect("a run command")
    }

    const PURE: InteropFlags = InteropFlags {
        interop_policy: None,
        pure: true,
    };

    #[test]
    fn run_consumes_a_pycc_flag_before_or_right_after_path() {
        // #1224: once `run` has flags, a pycc flag is recognized before
        // `PATH` and between `PATH` and the first forwarded value.
        for argv in [
            ["pycc", "run", "app.py", "--pure"],
            ["pycc", "run", "--pure", "app.py"],
        ] {
            let (interop, args) = run_interop(&argv);
            assert_eq!(interop, PURE, "{argv:?}");
            assert!(args.is_empty(), "{argv:?}");
        }
        assert!(parsed_run_interop(Command::Clean).is_none());
        let (interop, _) = run_interop(&["pycc", "run", "app.py", "--interop-policy", "allowlist"]);
        assert_eq!(interop.interop_policy, Some(InteropPolicy::Allowlist));
        assert_eq!(interop.into_cli().policy, Some(InteropPolicy::Allowlist));
        assert!(!interop.into_cli().pure);
    }

    #[test]
    fn run_forwards_a_pycc_flag_after_a_separator_or_a_forwarded_value() {
        let (interop, args) = run_interop(&["pycc", "run", "app.py", "--", "--pure"]);
        assert_eq!(interop, InteropFlags::default());
        assert_eq!(args, vec![OsString::from("--pure")]);

        let (interop, args) = run_interop(&["pycc", "run", "app.py", "x", "--pure"]);
        assert_eq!(interop, InteropFlags::default());
        assert_eq!(args, vec![OsString::from("x"), OsString::from("--pure")]);

        // Not a conflict: everything from the first forwarded value on is a
        // program argument, not a pycc flag, so D-128 rule 2's
        // order-independent conflict among parsed pycc flags is untouched.
        let (interop, args) = run_interop(&[
            "pycc",
            "run",
            "app.py",
            "--pure",
            "x",
            "--interop-policy",
            "deny",
        ]);
        assert_eq!(interop, PURE);
        assert_eq!(
            args,
            vec![
                OsString::from("x"),
                OsString::from("--interop-policy"),
                OsString::from("deny"),
            ]
        );

        // An unrecognized hyphen value is still forwarded, and so is every
        // pycc flag after it.
        let (interop, args) = run_interop(&["pycc", "run", "app.py", "--pur", "--pure"]);
        assert_eq!(interop, InteropFlags::default());
        assert_eq!(
            args,
            vec![OsString::from("--pur"), OsString::from("--pure")]
        );
    }

    #[test]
    fn pure_conflicts_with_every_explicit_policy_in_either_order() {
        for command in [
            &["build", "in.py", "-o", "out"][..],
            &["run", "in.py"],
            &["check", "in.py"],
        ] {
            for policy in ["auto", "allowlist", "deny"] {
                for flags in [
                    ["--pure", "--interop-policy", policy],
                    ["--interop-policy", policy, "--pure"],
                ] {
                    let argv: Vec<&str> = std::iter::once("pycc")
                        .chain(command.iter().copied())
                        .chain(flags)
                        .collect();
                    let error = Cli::try_parse_from(&argv).err().expect("a usage error");
                    assert_eq!(
                        error.kind(),
                        clap::error::ErrorKind::ArgumentConflict,
                        "{argv:?}"
                    );
                    assert_eq!(error.exit_code(), 2, "{argv:?}");
                }
            }
        }
    }

    #[test]
    fn ext_conflicts_with_both_interop_flags() {
        for flags in [&["--pure"][..], &["--interop-policy", "deny"]] {
            let argv: Vec<&str> = ["pycc", "build", "in.py", "-o", "out.so", "--ext"]
                .into_iter()
                .chain(flags.iter().copied())
                .collect();
            let error = Cli::try_parse_from(&argv).err().expect("a usage error");
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{argv:?}"
            );
        }
    }

    /// #1366: `--foreign-relative-imports` is an `--ext`-only opt-in, off
    /// by default, and parses in either order next to `--ext`.
    #[test]
    fn foreign_relative_imports_requires_ext() {
        let cli = Cli::try_parse_from(["pycc", "build", "in.py", "-o", "out", "--ext"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Build {
                foreign_relative_imports: false,
                ..
            }
        ));
        for order in [
            ["--ext", "--foreign-relative-imports"],
            ["--foreign-relative-imports", "--ext"],
        ] {
            let argv: Vec<&str> = ["pycc", "build", "in.py", "-o", "out"]
                .into_iter()
                .chain(order)
                .collect();
            let cli = Cli::try_parse_from(&argv).unwrap();
            assert!(matches!(
                cli.command,
                Command::Build {
                    ext: true,
                    foreign_relative_imports: true,
                    ..
                }
            ));
        }
        let error = Cli::try_parse_from([
            "pycc",
            "build",
            "in.py",
            "-o",
            "out",
            "--foreign-relative-imports",
        ])
        .err()
        .expect("a usage error");
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn build_accepts_static_libpython_and_rejects_it_with_ext() {
        let cli =
            Cli::try_parse_from(["pycc", "build", "in.py", "-o", "out", "--static-libpython"])
                .unwrap();
        assert!(matches!(
            cli.command,
            Command::Build {
                static_libpython: true,
                ..
            }
        ));
        let cli = Cli::try_parse_from(["pycc", "build", "in.py", "-o", "out"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Build {
                static_libpython: false,
                ..
            }
        ));
        for order in [
            ["--ext", "--static-libpython"],
            ["--static-libpython", "--ext"],
        ] {
            let argv: Vec<&str> = ["pycc", "build", "in.py", "-o", "out.so"]
                .into_iter()
                .chain(order)
                .collect();
            let error = Cli::try_parse_from(&argv).err().expect("a usage error");
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{argv:?}"
            );
            assert_eq!(error.exit_code(), 2, "{argv:?}");
        }
        let error = Cli::try_parse_from(["pycc", "run", "--static-libpython", "in.py"])
            .err()
            .expect("run has no such flag");
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn an_unknown_policy_and_a_repeated_pure_are_usage_errors() {
        let error = Cli::try_parse_from(["pycc", "check", "in.py", "--interop-policy", "strict"])
            .err()
            .expect("a usage error");
        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
        assert_eq!(error.exit_code(), 2);
        let error = Cli::try_parse_from(["pycc", "check", "in.py", "--pure", "--pure"])
            .err()
            .expect("a usage error");
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn build_and_check_accept_the_interop_flags() {
        let cli = Cli::try_parse_from(["pycc", "build", "in.py", "-o", "out", "--pure"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Build { interop, .. } if interop == PURE
        ));
        let cli =
            Cli::try_parse_from(["pycc", "check", "in.py", "--interop-policy", "deny"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Check { interop, .. }
                if interop.interop_policy == Some(InteropPolicy::Deny)
        ));
    }

    #[test]
    fn lock_parses_its_path_check_flag_and_interop_flags() {
        let cli = Cli::try_parse_from(["pycc", "lock", "app.py"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Lock { path, check: false, interop }
                if path == std::path::Path::new("app.py") && interop == InteropFlags::default()
        ));
        let cli = Cli::try_parse_from(["pycc", "lock", "--check", "app.py", "--pure"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Lock { check: true, interop, .. } if interop == PURE
        ));
        let error = Cli::try_parse_from(["pycc", "lock"])
            .err()
            .expect("a usage error");
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }
}
