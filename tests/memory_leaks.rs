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

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::Command;

const N_SMALL: u64 = 20_000;
const N_LARGE: u64 = 220_000;
const GROWTH_LIMIT_KB: i64 = 1024;

/// Helpers every probe body may call.
const PRELUDE: &str = r#"
from typing import Callable, Iterator


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


class P:
    def __init__(self, x: int, name: str) -> None:
        self.x = x
        self.name = name


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

fn build(dir: &Path, name: &str, source: &str) -> PathBuf {
    let src = dir.join(format!("{name}.py"));
    std::fs::write(&src, source).expect("write the probe source");
    let bin = dir.join(name);
    let output = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .arg("--release")
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
fn run_measured(bin: &Path) -> (Vec<u8>, i64) {
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

fn cpython(src: &Path) -> Vec<u8> {
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

// Generators: exhausted, abandoned unfinished (closed on release), and
// consumed by builtins; generator expressions.
probe!(generator_exhausted, "for v in gpair(i):\n    total += v");
probe!(
    generator_abandoned_holding_a_str,
    "g = gstr(i)\nt: str = next(g)\nif t == \"q\":\n    total += 1"
);
probe!(generator_listed, "total += len(list(gstr(i)))");
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

/// A list of exception instances holds them without a count: those objects
/// have none, and counting them corrupted their fields.
#[test]
fn a_list_of_exception_instances_leaves_them_intact() {
    let dir = ScratchDir::new("memory_fixture_exception_list").expect("scratch dir");
    let bin = assert_fixture_matches_cpython(&dir, "refcount_exception_list");
    assert_memcheck_clean(&bin);
}

#[test]
fn a_million_node_chain_is_released_without_recursing_and_a_cycle_is_harmless() {
    let dir = ScratchDir::new("memory_fixture_chain").expect("scratch dir");
    assert_fixture_matches_cpython(&dir, "refcount_deep_chain");
}

/// An `--ext` module called from a long-running CPython loop: every call's
/// strings, lists and instances are released, instances crossing in both
/// directions keep their identity, and nothing is freed while the host
/// still holds it. Read through the runtime's live-object counters, as
/// `tests/issue_1054_ext_str_release.rs` does.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_hosted_loop_calling_compiled_functions_runs_in_constant_memory() {
    let dir = ScratchDir::new("memory_ext_loop").expect("scratch dir");
    let src = dir.join("m.py");
    std::fs::write(&src, EXT_MODULE).expect("write the module source");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_memory_ext"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(EXT_DRIVER)
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success() && run.stdout == b"ok\n",
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

/// Freeing an instance releases the CPython object of its `object` slot,
/// and a function local read from the slot (`y = h.o`) owns a reference of
/// its own, so the object outlives the instance it was read from.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_local_read_from_an_object_slot_outlives_the_instance() {
    let dir = ScratchDir::new("memory_ext_object_slot").expect("scratch dir");
    let src = dir.join("m.py");
    std::fs::write(
        &src,
        "class Holder:\n    o: object\n\n    def __init__(self, o: object) -> None:\n        \
         self.o = o\n\n\ndef take(src: object) -> object:\n    h = Holder(src)\n    \
         h.o = src.copy()\n    y = h.o\n    h = Holder(src)\n    return y\n",
    )
    .expect("write the module source");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_memory_obj_slot"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let driver = "import pycc_memory_obj_slot as m\nfor _ in range(50):\n    \
                  r = m.take([1, 2, 3])\n    assert r == [1, 2, 3], r\nprint('ok')\n";
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(driver)
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success() && run.stdout == b"ok\n",
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

/// A list or instance temporary live when an exception unwinds is released
/// on the exception edge: a host loop catching compiled exceptions keeps
/// the runtime's live list and instance counts flat. (Exception objects and
/// their messages are leak-only, so `str` is not counted here.)
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn temporaries_live_when_an_exception_unwinds_are_released() {
    let dir = ScratchDir::new("memory_ext_unwind").expect("scratch dir");
    let src = dir.join("m.py");
    std::fs::write(&src, UNWIND_MODULE).expect("write the module source");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_memory_unwind"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(UNWIND_DRIVER)
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success() && run.stdout == b"ok\n",
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

const UNWIND_MODULE: &str = r#"class P:
    def __init__(self, x: int) -> None:
        self.x = x


class Q:
    def __init__(self, p: P, fail: bool) -> None:
        self.p = p
        if fail:
            raise ValueError("init")


def boom(n: int) -> int:
    if n >= 0:
        raise ValueError("boom")
    return n


def _boom_list(n: int) -> list[int]:
    if n >= 0:
        raise ValueError("boom")
    return [n]


def _take(xs: list[int], n: int) -> int:
    return len(xs) + n


def _mk(i: int) -> list[int]:
    return [i, i]


def operand(i: int) -> int:
    return len(_mk(i) + _boom_list(i))


def argument(i: int) -> int:
    return _take(_mk(i), boom(i))


def display(i: int) -> int:
    return len([_mk(i), _mk(i), _boom_list(i)])


def constructor(i: int) -> int:
    return Q(P(i), True).p.x


def element(i: int) -> int:
    return _mk(i)[i + 5]


def hold(i: int) -> int:
    xs = _mk(i)
    for x in xs:
        xs = [x]
        boom(i)
    return 0


def caught(i: int) -> int:
    total = 0
    for k in range(3):
        try:
            total += _take(_mk(i), boom(k))
        except ValueError:
            total += 1
    return total
"#;

const UNWIND_DRIVER: &str = r#"import ctypes, gc
import pycc_memory_unwind as m
lib = ctypes.CDLL(m.__file__)
def counter(name):
    f = getattr(lib, name)
    f.restype = ctypes.c_int64
    f.argtypes = []
    return f
counters = [counter('pycc_rt_%s_live_objects' % k) for k in ('list', 'instance')]
def live():
    gc.collect()
    return [c() for c in counters]
fns = [m.operand, m.argument, m.display, m.constructor, m.element, m.hold]
def round_trip():
    for f in fns:
        try:
            f(3)
        except (ValueError, IndexError):
            pass
        else:
            raise AssertionError(f.__name__)
    assert m.caught(1) == 3
round_trip()
before = live()
for _ in range(200):
    round_trip()
after = live()
assert after == before, 'live list/instance counts grew: %r -> %r' % (before, after)
print('ok')
"#;

const EXT_MODULE: &str = r#"class Cell:
    def __init__(self, v: int, label: str) -> None:
        self.v = v
        self.label = label

    def bump(self) -> "Cell":
        self.v = self.v + 1
        return self


class Pair:
    def __init__(self, a: Cell, b: Cell) -> None:
        self.a = a
        self.b = b


def churn(n: int) -> int:
    total = 0
    for i in range(n):
        xs = [Cell(i, f"c{i}"), Cell(i + 1, "d")]
        p = Pair(xs[0], xs[1])
        total += p.a.v + len(p.b.label)
    return total


def make(i: int) -> Cell:
    return Cell(i, f"m{i}")


def wrap(c: Cell) -> Pair:
    return Pair(c, Cell(0, "z"))


def same(c: Cell) -> Cell:
    return c
"#;

const EXT_DRIVER: &str = r#"import ctypes, gc
import pycc_memory_ext as m
lib = ctypes.CDLL(m.__file__)
def counter(name):
    f = getattr(lib, name)
    f.restype = ctypes.c_int64
    f.argtypes = []
    return f
counters = [counter('pycc_rt_%s_live_objects' % k) for k in ('str', 'list', 'instance')]
def live():
    gc.collect()
    return [c() for c in counters]
def round_trip():
    assert m.churn(50) == sum(i + 1 for i in range(50))
    c = m.make(3)
    assert m.same(c) is c and c.bump() is c and c.v == 4 and c.label == 'm3'
    p = m.wrap(c)
    del c
    assert p.a.v == 4 and p.b.label == 'z'
    host = m.Cell(7, 'host')
    assert m.same(host) is host and m.wrap(host).a is host
round_trip()
before = live()
for _ in range(300):
    round_trip()
after = live()
assert after == before, 'live str/list/instance counts grew: %r -> %r' % (before, after)
# A host-held instance lives exactly as long as its carrier.
h = m.Cell(1, 'held')
assert live()[2] == before[2] + 1
kept = m.wrap(h)
del h
assert kept.a.label == 'held'
del kept
assert live() == before, 'carrier release did not free: %r -> %r' % (before, live())
print('ok')
"#;
