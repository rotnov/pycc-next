//! Constant memory for the non-strict runtime paths the v0.5 corpus work
//! added: `itertools.accumulate`, `min` and `max` over a CPython object
//! (a list, or a native generator crossing as a CPython iterator), a
//! function returning `None` or a fresh list as a CPython object, and the
//! other CPython builtins over objects (`range` values, `tuple`, `iter`,
//! `next`, `list`, `sorted`, `sum`, `enumerate`, `itertools.chain`).
//!
//! Each probe runs its loop body `n` times for two `n` in non-strict mode;
//! the peak resident set may grow by at most [`GROWTH_LIMIT_KB`]
//! (`tests/memory_support` explains the bound), and the output must equal
//! CPython's. Each probe must also build without a CPython fallback, so
//! the native paths are the ones measured.
#![cfg(unix)]

#[path = "memory_support/mod.rs"]
mod memory_support;

use memory_support::{GROWTH_LIMIT_KB, cpython, run_measured};
use pycc_scratch::ScratchDir;
use std::process::Command;

const N_SMALL: u64 = 20_000;
const N_LARGE: u64 = 220_000;

const PRELUDE: &str = r#"
import itertools
from itertools import accumulate

PAIR = (10 ** 9, None)


def window(xs, a):
    yield PAIR
    while a < len(xs):
        yield xs[a]
        a += 1


def maybe_rows(n):
    if n % 3 == 0:
        return None
    rows = [[n] * 2 for _ in range(2)]
    rows[0][0] = n + 1
    return rows
"#;

fn program(body: &str, n: u64) -> String {
    let body = body.replace('\n', "\n        ");
    format!(
        "{PRELUDE}\n\ndef main():\n    total = 0\n    \
         for i in range({n}):\n        {body}\n    print(total)\n\n\nmain()\n"
    )
}

fn assert_constant_memory(name: &str, body: &str) {
    let dir = ScratchDir::new(&format!("memory_corpus_v05_{name}")).expect("scratch dir");
    let mut peaks = Vec::new();
    for n in [N_SMALL, N_LARGE] {
        let probe = format!("{name}_{n}");
        let source = dir.join(format!("{probe}.py"));
        std::fs::write(&source, program(body, n)).expect("write probe");
        let bin = dir.join(&probe);
        let built = Command::new(env!("CARGO_BIN_EXE_pycc"))
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(&bin)
            .args(["--release", "--non-strict", "--report-fallbacks"])
            .output()
            .expect("pycc should spawn");
        let report = String::from_utf8_lossy(&built.stderr);
        assert!(built.status.success(), "{name}: {report}");
        assert!(!report.contains("fallback:"), "{name}: {report}");
        let (out, peak) = run_measured(&bin);
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&cpython(&source)),
            "{name}: output differs from CPython"
        );
        peaks.push(peak);
    }
    let growth = peaks[1] - peaks[0];
    assert!(
        growth <= GROWTH_LIMIT_KB,
        "{name}: peak RSS grew by {growth} kB from {N_SMALL} to {N_LARGE} iterations \
         ({} -> {} kB): a value leaks every iteration",
        peaks[0],
        peaks[1]
    );
}

#[test]
fn accumulate_over_an_object_list() {
    assert_constant_memory(
        "accumulate",
        "xs = [i, \"a\"]\nxs[1] = 3\ntotal += list(accumulate(xs))[1]",
    );
}

#[test]
fn min_and_max_over_an_object_list() {
    assert_constant_memory(
        "min_max_list",
        "xs = [PAIR, (i, i)]\ntotal += min(xs)[0] + max(xs)[0]",
    );
}

#[test]
fn min_and_max_over_a_generator_of_objects() {
    assert_constant_memory(
        "min_max_generator",
        "xs = [PAIR, (i, i)]\ntotal += min(window(xs, 1))[0] + max(window(xs, 0))[0]",
    );
}

#[test]
fn a_none_or_list_return() {
    assert_constant_memory(
        "none_or_list",
        "rows = maybe_rows(i)\nif rows:\n    total += rows[0][0]\nelse:\n    total += 1",
    );
}

#[test]
fn cpython_builtins_over_objects() {
    assert_constant_memory(
        "builtins",
        "r = list(map(range, (2, i % 3 + 1)))\nr[0] = range(i % 5, 7)\nfor v in r[0]:\n    total += v\n\
         t = tuple(map(int, [str(i), \"2\"]))\ntotal += sum(t) + sorted(t)[0] + len(list(t))\n\
         g = iter([(i, 1), (2, i)])\na, b = next(g)\ntotal += min(a, b) + max(b, a)\n\
         xs = [None, i]\nxs[0] = 3\nfor k, v in enumerate(xs, 1):\n    total += k\n\
         total += len(list(itertools.chain([0], [p for p in xs])))",
    );
}
