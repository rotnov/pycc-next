//! Constant memory for the v0.4 language-gap shapes (ROADMAP v0.4 accept
//! 5): `IntEnum` members read as `int`, `dataclasses.asdict()`/`astuple()`/
//! `fields()`/`replace()`, a list of instances rendered by `__repr__`, and
//! a comprehension over instances returned from a function.
//! Each probe runs its loop body `n` times for two `n`; the peak resident
//! set may grow by at most [`GROWTH_LIMIT_KB`] (`tests/memory_leaks.rs`
//! explains the bound), and the output must equal CPython's.
#![cfg(unix)]

#[path = "memory_support/mod.rs"]
mod memory_support;

use memory_support::{build, cpython, run_measured};
use pycc_scratch::ScratchDir;

const N_SMALL: u64 = 20_000;
const N_LARGE: u64 = 220_000;
const GROWTH_LIMIT_KB: i64 = 1024;

const PRELUDE: &str = r#"
from dataclasses import asdict, astuple, dataclass, field, fields, replace
from enum import IntEnum


class Level(IntEnum):
    LOW = 1
    HIGH = 2


@dataclass
class P:
    x: int
    y: int


class Q:
    def __init__(self, x: int) -> None:
        self.x = x

    def tup(self) -> tuple[int, int]:
        return (self.x, 1)


def xs_of(qs: list[Q]) -> list[int]:
    return [q.x for q in qs]


@dataclass(order=True, frozen=True)
class Card:
    rank: int
    suit: str


class Grow:
    def __init__(self, n: int) -> None:
        self.n = n

    def __lt__(self, other: "Grow") -> bool:
        SINK[0].append(Grow(0))
        if self.n == 7:
            raise ValueError("seven")
        return self.n < other.n


SINK: list[list[Grow]] = [[]]


@dataclass(eq=False)
class Path:
    start: P
    hops: list[P]
    weights: dict[str, int] = field(default_factory=dict)
"#;

fn program(body: &str, n: u64) -> String {
    let body = body.replace('\n', "\n        ");
    format!(
        "{PRELUDE}\n\ndef main() -> None:\n    total: int = 0\n    \
         for i in range({n}):\n        {body}\n    print(total)\n\n\nmain()\n"
    )
}

fn assert_constant_memory(name: &str, body: &str) {
    let dir = ScratchDir::new(&format!("memory_v04_{name}")).expect("scratch dir");
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
    int_enum_members_read_as_ints,
    "xs = [i, i + 1, i + 2]\nif Level.HIGH > i % 3 and Level.LOW in xs:\n    total += 1\n\
     total += Level.HIGH * 2 + xs[Level.LOW] + int(Level.LOW)"
);
probe!(
    asdict_and_astuple_of_nested_dataclasses,
    "path = Path(P(i, 1), [P(1, 2), P(i, 3)], {\"w\": i})\nt = astuple(path)\n\
     d = asdict(P(i, 2))\ntotal += len(t[1]) + d[\"x\"] + len(t[2])"
);
probe!(
    a_returned_comprehension_over_instances,
    "total += len(xs_of([Q(i), Q(1)]))"
);
probe!(
    fields_and_replace,
    "p = replace(P(i, 0), y=i + 1)\ntotal += p.y + len(fields(p)) + len(fields(P)[1].name)"
);
probe!(
    a_list_of_instances_rendered_by_repr,
    "ps = [P(i, 1), P(2, i)]\ntotal += len(repr(ps)) + len(f\"{ps}\") + len(str([ps]))"
);
probe!(
    instances_sorted_and_found_in_a_set,
    "cards = [Card(i % 7, \"h\"), Card(3, \"s\"), Card(i % 5, \"c\")]\ncards.sort()\n\
     hand = {Card(3, \"s\"), Card(i % 4, \"d\")}\n\
     total += sorted(cards, reverse=True)[0].rank + min(cards).rank + max(Card(1, \"a\"), Card(2, \"b\")).rank\n\
     if Card(3, \"s\") in hand:\n    total += len(sorted(cards, key=lambda c: c))"
);
probe!(
    a_tuple_of_instances_rendered,
    "t = (P(i, 1), \"x\")\ntotal += len(str(t)) + len(repr([t]))"
);
probe!(
    a_list_changed_or_raising_while_sorted,
    "gs = [Grow(i % 9), Grow(3), Grow(5)]\nSINK[0] = gs\ntry:\n    gs.sort()\nexcept ValueError:\n    total += len(gs)\nSINK[0] = []"
);
