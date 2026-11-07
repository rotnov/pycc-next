//! Constant memory for the runtime paths the v0.3 corpus work added:
//! `next(sys.stdin)` (`pycc_rt_stdin_next`) reading every line of a long
//! input, including the `StopIteration` at its end. The probe runs for two
//! input sizes; the peak resident set may grow by at most
//! [`GROWTH_LIMIT_KB`] (`tests/memory_support` explains the bound, which
//! also covers the short lines of input itself), and the output must equal
//! CPython's.

#[path = "memory_support/mod.rs"]
mod memory_support;

use memory_support::build;
use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const N_SMALL: usize = 20_000;
const N_LARGE: usize = 220_000;
use memory_support::GROWTH_LIMIT_KB;

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
    let measured = memory_support::measure(Command::new(program), Some(input));
    assert!(
        measured.success(),
        "{} exited abnormally: {:#x}",
        program.display(),
        measured.status
    );
    (memory_support::lf(pycc_scratch::lf(&measured.stdout)), measured.peak_kb)
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
        let expected = memory_support::lf(oracle.wait_with_output().expect("CPython runs").stdout);
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

/// `any`/`all` of a generator of CPython objects (non-strict mode) and
/// iterating a homogeneous tuple of strings release every value they read,
/// as do `set.update`/`list.extend` of a generator and a generator passed
/// to a CPython object's method.
#[test]
#[ignore = "requires a CPython 3.13+ with a shared libpython (PYCC_PYTHON, default python3)"]
fn object_any_all_and_tuple_iteration_run_in_constant_memory() {
    let dir = ScratchDir::new("memory_corpus_any_all").expect("scratch dir");
    let mut peaks = Vec::new();
    for n in [N_SMALL, N_LARGE] {
        let name = format!("any_all_{n}");
        let src = dir.join(format!("{name}.py"));
        std::fs::write(
            &src,
            format!(
                "import json\n\nmixed = json.loads('[0, \"\", \"x\", [1]]')\nempty = json.loads('[]')\n\
                 words = (\"a\", \"bc\", \"def\")\ntotal = 0\nfor i in range({n}):\n    \
                 total += any(v for v in mixed) + all(v for v in mixed)\n    \
                 total += sum([len(w + \"!\") for w in words])\n    \
                 for w in words:\n        total += len(w * 2)\n    \
                 seen = {{\"a\"}}\n    seen.update(w + \"?\" for w in words)\n    \
                 xs = [\"x\"]\n    xs.extend(str(i) for i in range(3))\n    \
                 out = empty.copy()\n    out.extend(w * 2 for w in words)\n    \
                 total += len(seen) + len(xs) + len(out)\nprint(total)\n"
            ),
        )
        .expect("write the probe source");
        let bin = dir.join(&name);
        let output = Command::new(env!("CARGO_BIN_EXE_pycc"))
            .args(["build", "--release", "--non-strict", "--report-fallbacks"])
            .arg(&src)
            .arg("-o")
            .arg(&bin)
            .output()
            .expect("pycc should spawn");
        let report = pycc_scratch::lf(&*String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "{report}");
        assert!(!report.contains("fallback:"), "{report}");
        let (out, peak) = memory_support::run_measured(&bin);
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&memory_support::cpython(&src)),
            "output differs from CPython"
        );
        peaks.push(peak);
    }
    let growth = peaks[1] - peaks[0];
    assert!(
        growth <= GROWTH_LIMIT_KB,
        "peak RSS grew by {growth} kB from {N_SMALL} to {N_LARGE} iterations ({} -> {} kB)",
        peaks[0],
        peaks[1]
    );
}
