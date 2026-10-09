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

    let out_dir = env::var("OUT_DIR").unwrap_or_else(|err| panic!("OUT_DIR not set: {err}"));
    let evidence_path = std::path::Path::new(&out_dir).join("rustc_version.txt");
    fs::write(&evidence_path, version).unwrap_or_else(|err| {
        panic!("failed to write {}: {err}", evidence_path.display());
    });
}
