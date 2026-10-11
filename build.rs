//! Captures the actual compiler that built `pycc`, for `pycc version --verbose`.
//!
//! Cargo's `CARGO_PKG_RUST_VERSION` is the manifest's `rust-version` (the MSRV
//! contract), not the compiler that produced this binary -- a newer installed
//! `rustc` than the declared minimum builds cleanly and silently makes the two
//! diverge. See #247. This runs the same `rustc` Cargo itself is about to
//! invoke (`RUSTC`, falling back to `rustc` on `PATH` for tooling that doesn't
//! set it) and exposes its reported version through `PYCC_BUILD_RUSTC_VERSION`.
//!
//! Cargo only sets `RUSTC` for build script invocations, not for the test
//! binaries that later run in the same package -- so a version test can't
//! reliably re-resolve `RUSTC` itself at runtime and expect the same
//! compiler this script saw (e.g. when it's pinned via `.cargo/config.toml`
//! rather than the environment). This also writes the captured version to
//! `OUT_DIR/rustc_version.txt`, which `env!("OUT_DIR")` resolves to the same
//! directory in every target of this package, including integration tests --
//! giving them build-time evidence to check against without re-invoking
//! `rustc` themselves.
//!
//! The same is done for LLVM: `llvm-config --version` from
//! `LLVM_SYS_221_PREFIX` (the prefix `llvm-sys` itself builds against, so the
//! LLVM this binary links), falling back to `llvm-config` on `PATH`, becomes
//! `PYCC_BUILD_LLVM_VERSION` and `OUT_DIR/llvm_version.txt`.

use std::env;
use std::fs;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-env-changed=RUSTC");

    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let output = Command::new(&rustc)
        .arg("--version")
        .output()
        .unwrap_or_else(|err| panic!("failed to run `{rustc} --version`: {err}"));
    if !output.status.success() {
        panic!(
            "`{rustc} --version` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stdout = String::from_utf8(output.stdout)
        .unwrap_or_else(|err| panic!("`{rustc} --version` printed non-UTF-8 output: {err}"));
    // Expected shape: "rustc 1.97.1 (8bab26f4f 2026-07-14)\n" -- the second
    // whitespace-separated field is the bare version number, matching the
    // format `CARGO_PKG_RUST_VERSION` used to report.
    let version = stdout.split_whitespace().nth(1).unwrap_or_else(|| {
        panic!("unexpected `{rustc} --version` output: {stdout:?}");
    });
    println!("cargo::rustc-env=PYCC_BUILD_RUSTC_VERSION={version}");

    let llvm_version = llvm_version();
    println!("cargo::rustc-env=PYCC_BUILD_LLVM_VERSION={llvm_version}");

    let out_dir = env::var("OUT_DIR").unwrap_or_else(|err| panic!("OUT_DIR not set: {err}"));
    for (file, contents) in [
        ("rustc_version.txt", version),
        ("llvm_version.txt", llvm_version.as_str()),
    ] {
        let evidence_path = std::path::Path::new(&out_dir).join(file);
        fs::write(&evidence_path, contents).unwrap_or_else(|err| {
            panic!("failed to write {}: {err}", evidence_path.display());
        });
    }
}

/// The version `llvm-config` reports for the LLVM this build links. Panics
/// rather than guessing: `llvm-sys` needs the same `llvm-config` to build at
/// all, so a failure here is a broken toolchain, not a missing optional fact.
fn llvm_version() -> String {
    println!("cargo::rerun-if-env-changed=LLVM_SYS_221_PREFIX");
    let config = match env::var_os("LLVM_SYS_221_PREFIX") {
        Some(prefix) => {
            let exe = if cfg!(windows) {
                "llvm-config.exe"
            } else {
                "llvm-config"
            };
            std::path::Path::new(&prefix).join("bin").join(exe)
        }
        None => std::path::PathBuf::from("llvm-config"),
    };
    let output = Command::new(&config)
        .arg("--version")
        .output()
        .unwrap_or_else(|err| panic!("failed to run `{} --version`: {err}", config.display()));
    if !output.status.success() {
        panic!(
            "`{} --version` exited with {}",
            config.display(),
            output.status
        );
    }
    // Expected shape: "22.1.8\n"; packagers may append a suffix
    // ("22.1.8git"), which is kept since it is part of what was linked.
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        panic!("`{} --version` printed nothing", config.display());
    }
    version
}
