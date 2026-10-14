//! Constant memory for the runtime paths the v0.3 corpus work added:
//! `next(sys.stdin)` (`pycc_rt_stdin_next`) reading every line of a long
//! input, including the `StopIteration` at its end. The probe runs for two
//! input sizes; the peak resident set may grow by at most
//! [`GROWTH_LIMIT_KB`] (`tests/memory_leaks.rs` explains the bound, which
//! also covers the short lines of input itself), and the output must equal
//! CPython's.
#![cfg(unix)]

#[path = "memory_support/mod.rs"]
mod memory_support;

use memory_support::build;
use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const N_SMALL: usize = 20_000;
const N_LARGE: usize = 220_000;
const GROWTH_LIMIT_KB: i64 = 1024;

const PROBE: &str = "\
from sys import stdin


def main() -> None:
    total = 0
    lines = 0
    try:
        while True:
            total += len(next(stdin))
            lines += 1
    except StopIteration:
        pass
    print(lines, total)


main()
";

/// Runs `program` with `input` on its standard input; returns its stdout
/// and peak resident set in kilobytes.
fn run_with_input(program: &Path, input: &[u8]) -> (Vec<u8>, i64) {
    let mut child = Command::new(program)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the probe should spawn");
    let pid = child.id() as libc::pid_t;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut stdout, &mut out).expect("read the probe's stdout");
    writer
        .join()
        .expect("the writer thread")
        .expect("write the probe's stdin");
    let mut status = 0;
    // SAFETY: an all-zero `rusage` is a valid value for `wait4` to fill.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `pid` is our own unreaped child; both out-pointers are live.
    let waited = unsafe { libc::wait4(pid, &mut status, 0, &mut usage) };
    assert_eq!(waited, pid, "wait4 should reap the probe");
    assert!(
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
        "{} exited abnormally: {status:#x}",
        program.display()
    );
    let kb = if cfg!(target_os = "macos") {
        usage.ru_maxrss / 1024
    } else {
        usage.ru_maxrss
    };
    (out, kb)
}

#[test]
fn next_of_stdin_runs_in_constant_memory() {
    let dir = ScratchDir::new("memory_corpus_stdin_next").expect("scratch dir");
    let bin = build(&dir, "stdin_next", PROBE);
    let python = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let mut peaks = Vec::new();
    for n in [N_SMALL, N_LARGE] {
        let input = "7\n".repeat(n);
        let (out, peak) = run_with_input(&bin, input.as_bytes());
        let mut oracle = Command::new(&python)
            .arg(dir.join("stdin_next.py"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3 should spawn");
        oracle
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(input.as_bytes())
            .expect("write CPython's stdin");
        let expected = oracle.wait_with_output().expect("CPython runs").stdout;
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&expected)
        );
        peaks.push(peak);
    }
    let growth = peaks[1] - peaks[0];
    assert!(
        growth <= GROWTH_LIMIT_KB,
        "peak RSS grew by {growth} kB from {N_SMALL} to {N_LARGE} lines ({} -> {} kB)",
        peaks[0],
        peaks[1]
    );
}
