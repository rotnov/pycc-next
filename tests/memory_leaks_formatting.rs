//! Constant memory for the v0.4 corpus paths: format specs and printf-style
//! `%` formatting (`pycc_rt::format`), and in `--non-strict` builds a native
//! list collected from a comprehension over a CPython object, `set(o)` of an
//! object and an empty `set()`/`dict()` built as the CPython container, a
//! CPython `collections.defaultdict`, a chained comparison over objects, a
//! name widened to an object and a big `int` literal in a widened list. Each
//! probe runs its loop body `n` times for two `n`; the peak resident set may
//! grow by at most [`GROWTH_LIMIT_KB`] (`tests/memory_leaks.rs` explains the
//! bound), and the output must equal CPython's.

#[path = "memory_support/mod.rs"]
mod memory_support;

use memory_support::{build_with, cpython, run_measured};
use pycc_scratch::ScratchDir;

const N_SMALL: u64 = 20_000;
const N_LARGE: u64 = 220_000;
const GROWTH_LIMIT_KB: i64 = 1024;

const PRELUDE: &str = r#"
import collections


def pair(i):
    return [i, i + 1]
"#;

/// The probe program; a `--non-strict` one also binds `d`, a CPython
/// `OrderedDict`.
fn program(body: &str, n: u64, non_strict: bool) -> String {
    let body = body.replace('\n', "\n        ");
    let (prelude, object) = if non_strict {
        (PRELUDE, "d = collections.OrderedDict()\n    d[1] = 2\n    d[3] = 4\n    ")
    } else {
        ("", "")
    };
    format!(
        "{prelude}\n\ndef main() -> None:\n    total: int = 0\n    {object}\
         for i in range({n}):\n        {body}\n    print(total)\n\n\nmain()\n"
    )
}

fn assert_constant_memory(name: &str, body: &str, flags: &[&str]) {
    let dir = ScratchDir::new(&format!("memory_formatting_{name}")).expect("scratch dir");
    let mut peaks = Vec::new();
    for n in [N_SMALL, N_LARGE] {
        let probe = format!("{name}_{n}");
        let source = program(body, n, flags.contains(&"--non-strict"));
        let bin = build_with(&dir, &probe, &source, flags);
        let (out, peak) = run_measured(&bin);
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&cpython(&dir.join(format!("{probe}.py")))),
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

macro_rules! probe {
    ($name:ident, $flags:expr, $body:expr) => {
        #[test]
        #[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
        fn $name() {
            assert_constant_memory(stringify!($name), $body, $flags);
        }
    };
}

probe!(
    format_specs,
    &[],
    "x = i / 7\ntotal += len(f\"{i:>10,}|{x:.3f}|{x:e}\") + len(format(i, \"#x\")) + len(f\"{'ab':^6}\")"
);
probe!(
    printf_conversions,
    &[],
    "x = i / 3\ntotal += len('%5d|%-8.2f|%s|%r|%x' % (i, x, 'q', 'q', i)) + len('%.9f' % x)"
);
probe!(
    a_native_list_from_a_comprehension_over_an_object,
    &["--non-strict"],
    "rows = [[len(str(k)), 1] for k in d]\ntotal += len(rows) + rows[0][1] + len(pair(i))"
);
probe!(
    set_of_an_object_and_empty_object_containers,
    &["--non-strict"],
    "s = set(d)\ne = set()\nfor k in d:\n    e.add(k)\ntotal += len(s) + len(e) + len(dict())"
);
probe!(
    an_object_defaultdict,
    &["--non-strict"],
    "g = collections.defaultdict(list)\nfor k in d:\n    g[k].append(i)\ntotal += len(g) + len(g[1])"
);
probe!(
    a_chained_comparison_over_objects,
    &["--non-strict"],
    "for k in d:\n    if 0 < k < 10 > i % 7:\n        total += 1"
);
probe!(
    widened_names_and_a_big_literal,
    &["--non-strict"],
    "row = pair(i)\nd[5] = row\nrow.append(1)\nbig = [1]\nbig.append(1012809128301279797787789798789798798)\nif d != big:\n    total += len(d[5]) + len(big)"
);
