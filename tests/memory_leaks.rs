//! Native memory management end to end: a loop that creates and drops
//! values must run in constant memory.
//!
//! Each probe is one loop body, compiled into a program that runs it `n`
//! times. The program is built and run for two `n`, and the peak resident
//! set of the second run may exceed the first by no more than
//! [`GROWTH_LIMIT_KB`]. A leak of one 48-byte object per iteration over the
//! [`N_LARGE`]` - `[`N_SMALL`] extra iterations would be roughly ten
//! megabytes, so the limit separates a per-iteration leak from allocator
//! noise by two orders of magnitude. Every run's output is also compared
//! with CPython's for the same program.
#![cfg(unix)]

#[path = "memory_support/mod.rs"]
mod memory_support;

use memory_support::{build, cpython, run_measured};
use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::Command;

const N_SMALL: u64 = 20_000;
const N_LARGE: u64 = 220_000;
const GROWTH_LIMIT_KB: i64 = 1024;

/// Helpers every probe body may call.
const PRELUDE: &str = r#"
import collections
import itertools
from typing import Callable, Generator, Iterator


def mkadd(s: str, xs: list[int], n: int) -> Callable[[str], int]:
    def add(t: str) -> int:
        return len(s + t) + len(xs) + n
    return add


def call_it(f: Callable[[str], int], t: str) -> int:
    return f(t)


def gstr(i: int) -> Iterator[str]:
    try:
        yield mk(i)
        yield mk(i + 1)
    finally:
        D["a"] = i


def gpair(i: int) -> Iterator[int]:
    for x in mklist(i):
        yield x


def gecho(i: int) -> Generator[str, str, str]:
    got = mk(i)
    try:
        while len(got) < 9:
            got = yield got + "!"
    finally:
        D["e"] = i
    return got


def gdelegate(i: int) -> Generator[str, str, str]:
    first = yield from gecho(i)
    return first + mk(i)


def gfixed(i: int) -> Generator[str, None, str]:
    yield mk(i)
    yield "b"
    return mk(i + 1)


def gouter(i: int) -> Iterator[str]:
    v = yield from gfixed(i)
    yield v


def gcatch(i: int) -> Generator[str, None, None]:
    while True:
        try:
            yield mk(i)
        except GeneratorExit:
            D["c"] = i
            return


def sig_varargs(a: int, *rest: str, sep: str = ",", **named: str) -> int:
    return a + len(sep.join(rest)) + len(named)


class P:
    def __init__(self, x: int, name: str) -> None:
        self.x = x
        self.name = name


class Slot:
    def __init__(self, f: Callable[[str], int]) -> None:
        self.f = f


class Link:
    def __init__(self, v: int, nxt: P) -> None:
        self.v = v
        self.nxt = nxt


class Bag:
    items: list[P]

    def __init__(self) -> None:
        self.items = []

    def add(self, p: P) -> "Bag":
        self.items.append(p)
        return self


def ren(p: P) -> str:
    p.name = mk(1)
    return "x"


def ow(xs: list[str]) -> str:
    xs[0] = mk(2)
    return "y"


def swap_items(b: Bag) -> int:
    b.items = [mkp(5)]
    return 0


def find(xs: list[int]) -> int:
    for x in xs:
        xs = [0]
        if x > 0:
            return x
    return -1


def mkp(i: int) -> P:
    return P(i, mk(i))


def px(p: P) -> int:
    return p.x


def mklist(i: int) -> list[int]:
    return [i, i + 1]


def bump(v: int) -> int:
    w = v + 1
    return w


def mk(i: int) -> str:
    return f"v{i}"


def ident(s: str) -> str:
    return s


def take(s: str) -> int:
    return 1


class H:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        return self.v

    def __eq__(self, other: "H") -> bool:
        return self.v == other.v


class Tally:
    counts: dict[str, int]

    def __init__(self) -> None:
        self.counts = {}

    def add(self, k: str) -> "Tally":
        self.counts[k] = self.counts.get(k, 0) + 1
        return self


def mkd(i: int) -> dict[str, int]:
    return {mk(i): i, "z": 0}


D = {"a": 1}
NAMES = ["a", "b"]
"#;

fn program(body: &str, n: u64) -> String {
    let body = body.replace('\n', "\n        ");
    format!(
        "{PRELUDE}\n\ndef main() -> None:\n    total: int = 0\n    s: str = \"\"\n    \
         for i in range({n}):\n        {body}\n    print(total)\n\n\nmain()\n"
    )
}

/// Asserts that `body` runs in constant memory and prints what CPython
/// prints.
fn assert_constant_memory(name: &str, body: &str) {
    let dir = ScratchDir::new(&format!("memory_{name}")).expect("scratch dir");
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
        fn $name() {
            assert_constant_memory(stringify!($name), $body);
        }
    };
}

// `itertools` and `collections`, compiled natively.
#[path = "memory_leaks/stdlib.rs"]
mod stdlib;

// `str` temporaries consumed without being stored.
probe!(
    str_compared_with_a_literal,
    "if s == \"x\":\n    total += 1"
);
probe!(
    str_fstring_compared,
    "if f\"x{i}y\" == \"x5y\":\n    total += 1"
);
probe!(
    str_concatenation_operands,
    "if \"ab\" + f\"{i}\" + \"c\" == \"ab5c\":\n    total += 1"
);
probe!(
    str_repetition,
    "t: str = \"ab\" * 3\nif t == \"q\":\n    total += 1"
);
probe!(str_truth_test, "if mk(i):\n    total += 1");
probe!(str_discarded_call_result, "mk(i)");
probe!(
    str_or_operand,
    "t: str = mk(i) or \"x\"\nif t == \"q\":\n    total += 1"
);
probe!(
    str_conditional_expression,
    "t: str = mk(i) if i > 5 else \"x\"\nif t == \"q\":\n    total += 1"
);
probe!(
    str_argument_and_return,
    "t: str = ident(mk(i))\nif t < \"q\":\n    total += take(t)"
);
probe!(
    str_dict_key_lookup_and_rebind,
    "D[\"a\"] = i\ntotal += D[\"a\"] + D.get(mk(i), 0)"
);
probe!(str_membership_needle, "if mk(i) in NAMES:\n    total += 1");

// Lists: a fresh list per iteration, as a local, a temporary, an operand.
probe!(
    list_local_rebound,
    "xs: list[int] = [i, 2, 3]\ntotal += xs[0]"
);
probe!(
    list_temporary_subscript,
    "total += mklist(i)[1] + len(mklist(i))"
);
probe!(
    list_of_lists,
    "xs: list[list[int]] = [[i], [2, 3]]\ntotal += xs[1][0]"
);
probe!(
    list_of_str,
    "xs: list[str] = [mk(i), \"b\"]\nxs.append(\"c\")\nt: str = xs.pop()\nif t == \"q\":\n    total += 1"
);
probe!(
    list_slice_concat_repeat,
    "xs: list[int] = [i, 2, 3]\nys: list[int] = xs[1:] + [4] * 2\ntotal += ys[0]"
);
probe!(
    list_comprehension,
    "xs: list[int] = [j * i for j in range(3)]\ntotal += xs[1]"
);
probe!(
    list_rendered,
    "if i < 0:\n    print([i])\nt: str = str([i])\nif t == \"q\":\n    total += 1"
);
// Instances: locals, temporaries, arguments, attributes, containers.
probe!(instance_local, "p: P = P(i, mk(i))\ntotal += p.x");
probe!(
    instance_temporary_attribute,
    "total += mkp(i).x + len(mkp(i).name)"
);
probe!(instance_argument, "total += px(P(i, \"a\"))");
probe!(
    instance_attribute_chain,
    "k: Link = Link(i, P(i, \"z\"))\nk.nxt = P(1, \"y\")\ntotal += k.nxt.x"
);
probe!(
    instance_list,
    "ps: list[P] = [P(i, \"a\"), mkp(2)]\nfor q in ps:\n    total += q.x"
);
probe!(
    instance_holding_a_list,
    "b: Bag = Bag().add(P(i, \"a\")).add(mkp(i))\ntotal += len(b.items)"
);

// Operands pinned across a later sibling that runs code (`operand_pin`):
// the reference they take is released once per evaluation.
probe!(
    pinned_attribute_operand,
    "p: P = P(i, mk(i))\nif p.name == ren(p):\n    total += 1"
);
probe!(
    pinned_element_operand,
    "xs: list[str] = [mk(i)]\nif xs[0] + ow(xs) == \"q\":\n    total += 1"
);
probe!(
    pinned_subscript_base,
    "b: Bag = Bag().add(mkp(i))\ntotal += b.items[swap_items(b)].x"
);
// A `for` loop's own hold on a list it must keep is released by `return`.
probe!(
    for_loop_hold_released_on_return,
    "total += find(mklist(i + 1))"
);
// Dicts and sets: built per iteration, as locals, temporaries, attributes.
probe!(
    dict_local_rebound,
    "d: dict[str, int] = {mk(i): i, \"b\": 2}\nd[mk(i + 1)] = 3\ntotal += d[\"b\"]"
);
probe!(dict_temporary, "total += mkd(i)[\"z\"] + len(mkd(i))");
probe!(
    dict_comprehension,
    "d: dict[str, int] = {mk(j): j for j in range(3)}\ntotal += len(d)"
);
probe!(
    dict_loop_rebinding_its_dict,
    "d: dict[str, int] = mkd(i)\nfor k in d:\n    d = mkd(1)\n    total += len(k)"
);
probe!(
    dict_attribute,
    "t: Tally = Tally().add(mk(i)).add(\"x\").add(mk(i))\ntotal += len(t.counts)"
);
probe!(
    set_of_ints,
    "si: set[int] = {i, i + 1, 2}\ntotal += len(si)"
);
probe!(
    set_of_instances,
    "hs: set[H] = {H(i), H(i), H(2)}\nhs.add(H(2))\nhs.add(H(i + 7))\ntotal += len(hs)"
);
// A fresh target of a store dies once stored into.
probe!(fresh_attribute_target, "mkp(i).name = mk(i)");
probe!(fresh_list_target, "mklist(i)[0] = i");

// #625: a heap `int` passed to a function or a builtin, returned, and held
// by a function's parameter and locals when it returns.
probe!(
    bigint_through_a_function_and_builtins,
    "big: int = i + 2 ** 70\ntotal += bump(big * 2) % 7 + len(str(big * 3)) + max(big, big + 1) % 5"
);

// Function values capturing a `str`, a list and a big `int`: made, called,
// passed, rebound and called fresh.
probe!(
    closure_capturing_counted_values,
    "f = mkadd(mk(i), [i, i], i + 2 ** 70)\ntotal += f(\"ab\") + call_it(f, mk(i)) + mkadd(\"x\", [1], 1)(\"y\")"
);

// Function values capturing counted values, held by a list, a dict and an
// instance attribute, called from there, replaced, popped and printed.
probe!(
    closure_in_containers_and_attributes,
    "fs = [mkadd(mk(i), [i], i + 2 ** 70), mkadd(\"x\", [1], 1)]\nfs.append(fs[0])\ntotal += fs[2](\"ab\") + fs.pop()(\"c\")\nd: dict[str, Callable[[str], int]] = {\"a\": fs[1]}\nd[\"a\"] = mkadd(mk(i), [], 0)\ntotal += d[\"a\"](\"q\") + len(str(fs[0])) // 100\nh = Slot(fs[0])\nh.f = fs[1]\ntotal += h.f(mk(i))"
);

// Generators: exhausted, abandoned unfinished (closed on release), and
// consumed by builtins; generator expressions.
probe!(generator_exhausted, "for v in gpair(i):\n    total += v");
probe!(
    generator_abandoned_holding_a_str,
    "g = gstr(i)\nt: str = next(g)\nif t == \"q\":\n    total += 1"
);
probe!(generator_listed, "total += len(list(gstr(i)))");
// The generator protocol: values sent in, yielded through a delegation,
// returned from a subgenerator, and a generator closed while it holds
// counted locals.
probe!(
    generator_send_values,
    "g = gecho(i)\nt: str = next(g)\nt = g.send(mk(i))\nt = g.send(\"abc\")\ntotal += len(t)"
);
probe!(
    generator_delegation_returning_a_value,
    "d = gdelegate(i)\nt: str = next(d)\nt = d.send(mk(i + 1))\ntotal += len(t)\nd.close()"
);
probe!(
    generator_return_value_through_yield_from,
    "for t in gouter(i):\n    total += len(t)"
);
probe!(
    generator_closed_explicitly,
    "c = gcatch(i)\nt: str = next(c)\nc.close()\ntotal += len(t)"
);
// Calls binding `*args`, keyword-only parameters and `**kwargs`, statically
// and through `*seq`/`**mapping`.
probe!(
    signature_varargs_and_kwargs,
    "total += sig_varargs(1, mk(i), \"b\", sep=mk(2), x=mk(3))\nws: list[str] = [mk(i), \"q\"]\nkw: dict[str, str] = {\"y\": mk(i)}\ntotal += sig_varargs(i, *ws, **kw)"
);
probe!(
    generator_expression_short_circuit,
    "if any(x > i for x in mklist(i)):\n    total += 1\ntotal += next(len(t) for t in gstr(i))"
);

/// Builds and runs a fixture under `tests/fixtures/` in both profiles and
/// compares its output with CPython's, returning the release binary's path
/// inside `dir`.
fn assert_fixture_matches_cpython(dir: &Path, name: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.py"));
    let expected = cpython(&src);
    let mut release_bin = PathBuf::new();
    for release in [false, true] {
        let bin = dir.join(if release { "release" } else { "debug" });
        let mut build = Command::new(env!("CARGO_BIN_EXE_pycc"));
        build.arg("build").arg(&src).arg("-o").arg(&bin);
        if release {
            build.arg("--release");
        }
        let output = build.output().expect("pycc should spawn");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let (out, _) = run_measured(&bin);
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&expected),
            "{name}: output differs from CPython"
        );
        release_bin = bin;
    }
    release_bin
}

/// Runs `bin` under valgrind's memcheck when valgrind is installed: no
/// invalid read, write or free. Skipped (with a note) otherwise.
fn assert_memcheck_clean(bin: &Path) {
    let Ok(output) = Command::new("valgrind")
        .args(["-q", "--error-exitcode=99", "--leak-check=no"])
        .arg(bin)
        .output()
    else {
        eprintln!(
            "valgrind not installed: memcheck skipped for {}",
            bin.display()
        );
        return;
    };
    assert!(
        output.status.success(),
        "memcheck errors in {}:\n{}",
        bin.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn aliasing_and_release_order_match_cpython_and_are_memcheck_clean() {
    let dir = ScratchDir::new("memory_fixture_semantics").expect("scratch dir");
    let bin = assert_fixture_matches_cpython(&dir, "refcount_semantics");
    assert_memcheck_clean(&bin);
}

/// An operand read out of an attribute or element stays alive while a
/// later operand of the same operation runs code that drops its owner.
#[test]
fn operands_outlive_later_siblings_that_drop_their_owner() {
    let dir = ScratchDir::new("memory_fixture_operands").expect("scratch dir");
    let bin = assert_fixture_matches_cpython(&dir, "refcount_operand_lifetime");
    assert_memcheck_clean(&bin);
}

/// `xs += ys` / `xs *= n` mutate the list every alias sees, and their
/// element references balance; so do `heapq`/`bisect` on lists.
#[test]
fn in_place_list_operators_and_heaps_are_memcheck_clean() {
    let dir = ScratchDir::new("memory_fixture_list_inplace").expect("scratch dir");
    let bin = assert_fixture_matches_cpython(&dir, "corpus_list_inplace");
    assert_memcheck_clean(&bin);
    let dir = ScratchDir::new("memory_fixture_list_imul").expect("scratch dir");
    let bin = assert_fixture_matches_cpython(&dir, "corpus_list_imul");
    assert_memcheck_clean(&bin);
    let dir = ScratchDir::new("memory_fixture_heap_bisect").expect("scratch dir");
    let bin = assert_fixture_matches_cpython(&dir, "corpus_heap_bisect");
    assert_memcheck_clean(&bin);
}

/// Dicts and sets are reference counted: aliasing, rebinding, a loop over a
/// container its body rebinds, instance sets with duplicate inserts.
#[test]
fn dicts_and_sets_match_cpython_and_are_memcheck_clean() {
    let dir = ScratchDir::new("memory_fixture_dict_set").expect("scratch dir");
    let bin = assert_fixture_matches_cpython(&dir, "refcount_dict_set");
    assert_memcheck_clean(&bin);
}

/// Generic dicts, sets and tuple objects (any key, value and element type),
/// lifted closures, `key=` sorting and a `KeyError` from a dict of tuples:
/// every container owns and releases what it stores.
#[test]
fn generic_containers_match_cpython_and_are_memcheck_clean() {
    for fixture in ["closures_keys"] {
        let dir = ScratchDir::new("memory_fixture_generic").expect("scratch dir");
        let bin = assert_fixture_matches_cpython(&dir, fixture);
        assert_memcheck_clean(&bin);
    }
}

/// A list of exception instances counts them as exceptions (kind `x`),
/// never as class instances: counting them as instances corrupted their
/// fields.
#[test]
fn a_list_of_exception_instances_leaves_them_intact() {
    let dir = ScratchDir::new("memory_fixture_exception_list").expect("scratch dir");
    let bin = assert_fixture_matches_cpython(&dir, "refcount_exception_list");
    assert_memcheck_clean(&bin);
}

/// Exception objects are reference counted: a caught exception kept in a
/// list or a slot, returned from its handler, re-raised, chained, set
/// aside by `finally`, thrown into a generator or split by `except*`
/// stays alive exactly as long as CPython keeps it, and is never read
/// after it is freed.
#[test]
fn exception_lifetimes_match_cpython_and_are_memcheck_clean() {
    let dir = ScratchDir::new("memory_fixture_exception_lifetime").expect("scratch dir");
    let bin = assert_fixture_matches_cpython(&dir, "exception_lifetime");
    assert_memcheck_clean(&bin);
}

#[test]
fn a_million_node_chain_is_released_without_recursing_and_a_cycle_is_harmless() {
    let dir = ScratchDir::new("memory_fixture_chain").expect("scratch dir");
    assert_fixture_matches_cpython(&dir, "refcount_deep_chain");
}

/// Non-strict mode: a CPython object read as an index (`xs[o]`,
/// `range(o)`) is encoded as an inline word -- an index beyond any
/// sequence clamps into the sequence's own `IndexError` -- so a loop that
/// indexes with an object leaks nothing, and a huge index raises what
/// CPython raises.
#[test]
#[ignore = "requires a CPython 3.13+ with a shared libpython (PYCC_PYTHON, default python3)"]
fn an_object_index_runs_in_constant_memory() {
    let dir = ScratchDir::new("memory_object_index").expect("scratch dir");
    let mut peaks = Vec::new();
    for n in [N_SMALL, N_LARGE] {
        let name = format!("object_index_{n}");
        let src = dir.join(format!("{name}.py"));
        std::fs::write(
            &src,
            format!(
                "import json\n\nbig = json.loads(\"9\" * 40)\nsmall = json.loads(\"2\")\n\
                 xs = [1, 2, 3]\ns = \"abc\"\ntotal = 0\nfor i in range({n}):\n    \
                 total += xs[small] + len(s[small]) + xs[-small]\n    \
                 for j in range(small):\n        total += j\n\
                 print(total)\ntry:\n    print(xs[big])\nexcept IndexError as e:\n    print(e)\n\
                 try:\n    print(s[-big])\nexcept IndexError as e:\n    print(e)\n\
                 try:\n    print(xs[big * big])\nexcept IndexError as e:\n    print(e)\n"
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
        let report = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{report}");
        assert!(!report.contains("fallback:"), "{report}");
        let (out, peak) = run_measured(&bin);
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&cpython(&src)),
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

/// Non-strict mode: a comprehension collecting CPython objects over a native
/// source, or over a name bound to a CPython object, builds a CPython list
/// or set each trip; the collection, the packed elements and the operands
/// are all released.
#[test]
#[ignore = "requires a CPython 3.13+ with a shared libpython (PYCC_PYTHON, default python3)"]
fn an_object_comprehension_runs_in_constant_memory() {
    let dir = ScratchDir::new("memory_object_comprehension").expect("scratch dir");
    let mut peaks = Vec::new();
    for n in [N_SMALL, N_LARGE] {
        let name = format!("object_comprehension_{n}");
        let src = dir.join(format!("{name}.py"));
        std::fs::write(
            &src,
            format!(
                "import json\n\nx = json.loads(\"2\")\nobjs = json.loads(\"[1, 2, 3]\")\n\
                 total = 0\nfor i in range({n}):\n    \
                 ys = [j * x for j in range(20) if j != 3]\n    \
                 zs = {{x + j % 4 for j in range(8)}}\n    \
                 ws = [o + x for o in objs]\n    \
                 total += len(ys) + len(zs) + len(ws) + len([0] * x)\n\
                 print(total)\n"
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
        let report = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{report}");
        assert!(!report.contains("fallback:"), "{report}");
        let (out, peak) = run_measured(&bin);
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&cpython(&src)),
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

/// Non-strict mode: `str % args` formats through CPython, and
/// `sys.stdout.write` converts the CPython `str` it gets; neither the
/// formatted object nor the converted string outlives its statement.
#[test]
#[ignore = "requires a CPython 3.13+ with a shared libpython (PYCC_PYTHON, default python3)"]
fn percent_formatting_runs_in_constant_memory() {
    let dir = ScratchDir::new("memory_percent_format").expect("scratch dir");
    let mut peaks = Vec::new();
    for n in [N_SMALL, N_LARGE] {
        let name = format!("percent_format_{n}");
        let src = dir.join(format!("{name}.py"));
        std::fs::write(
            &src,
            format!(
                "import sys\n\ntotal = 0\nfor i in range({n}):\n    \
                 s = '%d-%s|%.3f' % (i, 'ab' * 40, i / 7)\n    \
                 total += len(s)\n    \
                 if i % 50000 == 0:\n        sys.stdout.write('%d ' % i)\n\
                 print(total)\n"
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
        let report = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{report}");
        assert!(!report.contains("fallback:"), "{report}");
        let (out, peak) = run_measured(&bin);
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&cpython(&src)),
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

/// Non-strict mode: keyed sorting, `min`/`max` and `heapq`/`bisect` over a
/// CPython object (`object_stdlib.c`) release the pairs list, the boxed
/// arguments and the results.
#[test]
#[ignore = "requires a CPython 3.13+ with a shared libpython (PYCC_PYTHON, default python3)"]
fn object_stdlib_calls_run_in_constant_memory() {
    let dir = ScratchDir::new("memory_object_stdlib").expect("scratch dir");
    let mut peaks = Vec::new();
    for n in [N_SMALL, N_LARGE] {
        let name = format!("object_stdlib_{n}");
        let src = dir.join(format!("{name}.py"));
        std::fs::write(
            &src,
            format!(
                "import heapq\nimport json\nfrom bisect import bisect_left\n\n\
                 xs = json.loads(\"[[3, 5], [1, 9], [2, 4]]\")\nh = json.loads(\"[]\")\n\
                 total = 0\nfor i in range({n}):\n    \
                 xs.sort(key=lambda t: t[1] - t[0], reverse=i % 2 == 0)\n    \
                 ys = sorted(xs, key=lambda t: t[0])\n    \
                 heapq.heappush(h, (i % 7, i))\n    \
                 if len(h) > 5:\n        heapq.heappop(h)\n    \
                 total += bisect_left(ys, xs[0]) + len(min(xs, key=len))\n\
                 print(total, xs, h)\n"
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
        let report = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{report}");
        assert!(!report.contains("fallback:"), "{report}");
        let (out, peak) = run_measured(&bin);
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&cpython(&src)),
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
