//! The peak-RSS probe harness shared by `tests/memory_leaks*.rs`: build a
//! probe with `pycc build --release`, run it measuring its peak resident
//! set, and run CPython on the same source.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn build(dir: &Path, name: &str, source: &str) -> PathBuf {
    build_with(dir, name, source, &[])
}

/// [`build`] with extra `pycc build` flags (`--non-strict`).
pub fn build_with(dir: &Path, name: &str, source: &str, flags: &[&str]) -> PathBuf {
    let src = dir.join(format!("{name}.py"));
    std::fs::write(&src, source).expect("write the probe source");
    let bin = dir.join(name);
    let output = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .arg("--release")
        .args(flags)
        .output()
        .expect("pycc should spawn");
    assert!(
        output.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    bin
}

/// Runs `bin`, returning its stdout and peak resident set in kilobytes.
pub fn run_measured(bin: &Path) -> (Vec<u8>, i64) {
    let child = Command::new(bin)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the probe should spawn");
    let pid = child.id() as libc::pid_t;
    let mut stdout = child.stdout.expect("piped stdout");
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut stdout, &mut out).expect("read the probe's stdout");
    let mut status = 0;
    // SAFETY: an all-zero `rusage` is a valid value for `wait4` to fill.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `pid` is our own unreaped child; both out-pointers are live.
    let waited = unsafe { libc::wait4(pid, &mut status, 0, &mut usage) };
    assert_eq!(waited, pid, "wait4 should reap the probe");
    assert!(
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
        "{} exited abnormally: {status:#x}",
        bin.display()
    );
    let max_rss = usage.ru_maxrss;
    // `ru_maxrss` is in bytes on macOS and in kilobytes elsewhere.
    let kb = if cfg!(target_os = "macos") {
        max_rss / 1024
    } else {
        max_rss
    };
    (out, kb)
}

pub fn cpython(src: &Path) -> Vec<u8> {
    let output = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg(src)
        .output()
        .expect("python3 should spawn");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
