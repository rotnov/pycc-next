//! Constant memory for the stdlib shapes compiled natively: `@dataclass`
//! instances (built, compared, rendered with `repr()`, a frozen one hashed
//! and stored in a set), enum members (rendered, looked up by value, used as
//! dict keys) and a `functools.lru_cache` table that evicts. Each probe runs
//! its loop body `n` times for two `n`; the peak resident set may grow by at
//! most [`GROWTH_LIMIT_KB`] (`tests/memory_support` explains the bound),
//! and the output must equal CPython's.
#![cfg(unix)]

#[path = "memory_support/mod.rs"]
mod memory_support;

use memory_support::{build, cpython, run_measured};
use pycc_scratch::ScratchDir;

const N_SMALL: u64 = 20_000;
const N_LARGE: u64 = 220_000;
use memory_support::GROWTH_LIMIT_KB;

const PRELUDE: &str = r#"
from dataclasses import dataclass, field
from enum import Enum, IntEnum
from functools import lru_cache


@dataclass(order=True)
class P:
    x: int
    name: str = "p"
    tags: list[int] = field(default_factory=list)


@dataclass(frozen=True)
class K:
    a: int
    b: str


class Color(Enum):
    RED = 1
    GREEN = 2


class Level(IntEnum):
    LOW = 1
    HIGH = 2


@lru_cache(maxsize=8)
def label(n: int) -> str:
    return "n" + str(n)
"#;

fn program(body: &str, n: u64) -> String {
    let body = body.replace('\n', "\n        ");
    format!(
        "{PRELUDE}\n\ndef main() -> None:\n    total: int = 0\n    \
         for i in range({n}):\n        {body}\n    print(total)\n\n\nmain()\n"
    )
}

fn assert_constant_memory(name: &str, body: &str) {
    let dir = ScratchDir::new(&format!("memory_stdlib_{name}")).expect("scratch dir");
    let mut peaks = Vec::new();
    for n in [N_SMALL, N_LARGE] {
        let probe = format!("{name}_{n}");
        let bin = build(&dir, &probe, &program(body, n));
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
    ($name:ident, $body:expr) => {
        #[test]
        #[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
        fn $name() {
            assert_constant_memory(stringify!($name), $body);
        }
    };
}

probe!(
    dataclass_built_compared_and_rendered,
    "p = P(i, \"q\")\nif p == P(i, \"q\") and p < P(i + 1):\n    total += len(repr(p)) + len(str(p))"
);
probe!(
    dataclass_default_factory_list,
    "p = P(i)\np.tags.append(i)\ntotal += len(p.tags) + len(f\"{p!r}\")"
);
probe!(
    frozen_dataclass_hashed_into_a_set,
    "s = {K(i % 3, \"x\"), K(1, \"x\")}\ntotal += len(s) + (1 if hash(K(i, \"b\")) else 0)"
);
probe!(
    enum_members_rendered_looked_up_and_keyed,
    "c = Color(i % 2 + 1)\nd = {c: str(c), Color.RED: repr(Level(i % 2 + 1))}\ntotal += len(d[c]) + len(d)"
);
probe!(
    an_evicting_lru_cache,
    "total += len(label(i % 20))"
);
