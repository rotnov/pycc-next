//! Constant memory for native values handed to CPython: a fresh `str`,
//! list, bigint `int` or class instance passed as a positional or keyword
//! argument of a call on a CPython function or method, or used as an
//! object's subscript key. The packer copies the native value into a
//! CPython object, so the native temporary is still the compiled code's to
//! release; before `pycc_codegen`'s `native_operand` it leaked once per
//! call (`json.dumps(str(i))` grew ~47 bytes per iteration).
//!
//! Each probe runs its loop body `n` times for two `n`, in strict and in
//! non-strict mode; the peak resident set may grow by at most
//! [`GROWTH_LIMIT_KB`] (`tests/memory_support` explains the bound), and
//! the output must equal CPython's.
#![cfg(unix)]

#[path = "memory_support/mod.rs"]
mod memory_support;

use memory_support::{build_with, cpython, run_measured};
use pycc_scratch::ScratchDir;

const N_SMALL: u64 = 20_000;
const N_LARGE: u64 = 220_000;
use memory_support::GROWTH_LIMIT_KB;

const PRELUDE: &str = r#"
import json
import os.path
import re


class Node:
    def __init__(self, v: int) -> None:
        self.v = v
        self.s = "n" + str(v)
"#;

fn program(body: &str, n: u64) -> String {
    let body = body.replace('\n', "\n        ");
    format!(
        "{PRELUDE}\n\ndef main() -> None:\n    total: int = 0\n    \
         for i in range({n}):\n        {body}\n    print(total)\n\n\nmain()\n"
    )
}

fn assert_constant_memory(name: &str, body: &str, sizes: [u64; 2]) {
    assert_constant_memory_in(name, body, "strict", &[], sizes);
    assert_constant_memory_in(name, body, "nonstrict", &["--non-strict"], sizes);
}

fn assert_constant_memory_in(name: &str, body: &str, mode: &str, flags: &[&str], sizes: [u64; 2]) {
    let dir = ScratchDir::new(&format!("memory_foreign_{name}_{mode}")).expect("scratch dir");
    let mut peaks = Vec::new();
    for n in sizes {
        let probe = format!("{name}_{n}");
        let bin = build_with(&dir, &probe, &program(body, n), flags);
        let (out, peak) = run_measured(&bin);
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&cpython(&dir.join(format!("{probe}.py")))),
            "{name} ({mode}): output differs from CPython"
        );
        peaks.push(peak);
    }
    let growth = peaks[1] - peaks[0];
    assert!(
        growth <= GROWTH_LIMIT_KB,
        "{name} ({mode}): peak RSS grew by {growth} kB from {} to {} \
         iterations ({} -> {} kB): a value leaks every iteration",
        sizes[0],
        sizes[1],
        peaks[0],
        peaks[1]
    );
}

macro_rules! probe {
    ($name:ident, $body:expr) => {
        probe!($name, $body, [N_SMALL, N_LARGE]);
    };
    ($name:ident, $body:expr, $sizes:expr) => {
        #[test]
        #[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
        fn $name() {
            assert_constant_memory(stringify!($name), $body, $sizes);
        }
    };
}

probe!(
    fresh_str_argument_of_a_cpython_function,
    "total += len(json.dumps(str(i)))"
);
probe!(
    str_literal_argument_of_a_cpython_function,
    "total += len(json.dumps(\"ab\"))"
);
probe!(
    concatenated_and_formatted_str_arguments,
    "total += len(os.path.join(\"a\" + str(i), f\"x{i}\"))"
);
probe!(
    fresh_str_keyword_argument,
    "total += len(json.dumps(str(i), indent=None))"
);
probe!(
    fresh_str_arguments_of_a_cpython_method,
    "d = json.loads(\"{\\\"1\\\": 2}\")\ntotal += len(str(d.get(str(i % 3), 0)))\n\
     total += len(re.sub(\"1\", \"x\" + str(i), str(i)))"
);
probe!(
    fresh_str_subscript_key,
    "d = json.loads(\"{\\\"1\\\": 2}\")\ntotal += len(str(d[str(i % 1 + 1)]))"
);
// Measured from `N_LARGE` on: CPython's own caches for `json.dumps` of a
// large `int` keep growing past `N_SMALL` (about 1 MB more by `N_LARGE`, and
// under 100 kB more from `N_LARGE` to ten times it), while a leaked object
// per iteration would grow by tens of megabytes over that span.
probe!(
    bigint_argument_of_a_cpython_function,
    "total += len(json.dumps(i * 100000000000000000000))",
    [N_LARGE, 10 * N_LARGE]
);
probe!(
    tuple_argument_of_a_cpython_function,
    "total += len(json.dumps((i, 2.5, True)))"
);
probe!(
    instance_argument_kept_by_cpython,
    "l = json.loads(\"[]\")\nl.append(Node(i))\nl.append(Node(i + 1))\n\
     total += len(l) + len(l[1].s)"
);

/// A native list crosses only in non-strict mode (strict refuses it with
/// `I0404`), so this probe runs that mode alone.
#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn fresh_list_arguments_non_strict() {
    assert_constant_memory_in(
        "fresh_list_arguments",
        "total += len(json.dumps([str(i), \"x\"])) + len(json.dumps([i, i]))",
        "nonstrict",
        &["--non-strict"],
        [N_SMALL, N_LARGE],
    );
}
