//! Exception objects are reference counted (`pycc_rt::exception::lifetime`):
//! a loop that raises and catches runs in constant memory, whatever holds
//! the exception on the way -- a handler and its `as` binding, a list or
//! an instance slot, a `__cause__`, a `finally` clause that set it aside, a
//! generator suspended in a handler, an `except*` partition, a
//! `StopIteration` carrying a generator's return value -- and whether the
//! exception is native or bridged from a CPython call.
//!
//! Each probe runs its loop body `n` times for two `n`, in strict and in
//! non-strict mode; the peak resident set may grow by at most
//! [`GROWTH_LIMIT_KB`] (`tests/memory_support` explains the bound) and the
//! output must equal CPython's. The counts are ten times
//! `tests/memory_leaks.rs`' so that a small leak stands far above the noise
//! of a program embedding CPython (non-strict mode, or `import json`): ten
//! bytes per iteration grow the peak by about twenty megabytes, and an
//! exception leaked with its message is over a hundred bytes. Before
//! exceptions were counted, catching `ZeroDivisionError` grew by about 220
//! bytes per iteration and catching a failing `json.loads` by about 1.9 kB.
#![cfg(unix)]

#[path = "memory_support/mod.rs"]
mod memory_support;

use memory_support::{build_with, cpython, run_measured};
use pycc_scratch::ScratchDir;

const N_SMALL: u64 = 200_000;
const N_LARGE: u64 = 2_200_000;
use memory_support::GROWTH_LIMIT_KB;

/// Helpers every probe body may call.
const PRELUDE: &str = r#"
from typing import Generator, Iterator


class AppError(Exception):
    pass


class DbError(AppError):
    pass


class Keeper:
    def __init__(self, err: ValueError) -> None:
        self.err = err


D = {"a": 1}


def fail(n: int) -> int:
    if n > 0:
        raise ValueError("bad " + str(n))
    return n


def raise_db(n: int) -> None:
    raise DbError("db " + str(n))


def caught_in_callee(n: int) -> str:
    s = "s" + str(n)
    try:
        fail(n)
    except ValueError as e:
        return s + f"{e}"
    return s


def keep_one(n: int) -> ValueError:
    try:
        fail(n)
    except ValueError as e:
        return e
    raise RuntimeError("unreachable")


def reraise(n: int) -> None:
    try:
        fail(n)
    except ValueError as e:
        if len(f"{e}") > 100:
            return
        raise


def chained(n: int) -> None:
    try:
        fail(n)
    except ValueError as e:
        raise AppError("wrapped " + str(n)) from e


def finally_replaces(n: int) -> int:
    try:
        fail(n + 1)
    finally:
        raise KeyError("replaced " + str(n))


def through_finally(n: int) -> int:
    s = "t" + str(n)
    try:
        return fail(n)
    finally:
        s = s + "!"


def break_out(n: int) -> int:
    count = 0
    for i in range(n):
        try:
            fail(i)
        except ValueError:
            break
        count += 1
    return count


def continue_on(n: int) -> int:
    count = 0
    for i in range(n):
        try:
            fail(i % 2)
        except ValueError:
            continue
        count += 1
    return count


def gen(n: int) -> Generator[int, None, str]:
    yield n
    return "done " + str(n)


def stop_value(n: int) -> str:
    g = gen(n)
    next(g)
    try:
        next(g)
    except StopIteration as e:
        return e.value
    return "no stop"


def counting() -> Iterator[int]:
    try:
        yield 1
        yield 2
    except ValueError as e:
        yield len(f"{e}")


def suspended_in_handler(n: int) -> Iterator[int]:
    try:
        fail(n)
    except ValueError as e:
        yield len(f"{e}")
        yield 0


def groups(n: int) -> int:
    count = 0
    try:
        fail(n)
    except ValueError as v:
        try:
            raise KeyError("k")
        except KeyError as k:
            try:
                raise ExceptionGroup("many", [v, k])
            except* ValueError:
                count += 1
            except* KeyError:
                count += 2
    return count


def deep(n: int) -> int:
    s = "d" + str(n)
    xs = [s, s]
    return len(xs) + fail(n)
"#;

fn program(imports: &str, body: &str, n: u64) -> String {
    let body = body.replace('\n', "\n        ");
    format!(
        "{imports}{PRELUDE}\n\ndef main() -> None:\n    total: int = 0\n    \
         for i in range({n}):\n        {body}\n    print(total)\n\n\nmain()\n"
    )
}

fn assert_constant_memory(name: &str, imports: &str, body: &str) {
    assert_constant_memory_in(name, imports, body, "strict", &[]);
    assert_constant_memory_in(name, imports, body, "nonstrict", &["--non-strict"]);
}

fn assert_constant_memory_in(name: &str, imports: &str, body: &str, mode: &str, flags: &[&str]) {
    let dir = ScratchDir::new(&format!("memory_exc_{name}_{mode}")).expect("scratch dir");
    let mut peaks = Vec::new();
    for n in [N_SMALL, N_LARGE] {
        let probe = format!("{name}_{n}");
        let bin = build_with(&dir, &probe, &program(imports, body, n), flags);
        let (out, peak) = run_measured(&bin);
        // CPython runs the smaller count only: the larger one would take it
        // minutes, and the same program text is what both counts share.
        if n == N_SMALL {
            assert_eq!(
                String::from_utf8_lossy(&out),
                String::from_utf8_lossy(&cpython(&dir.join(format!("{probe}.py")))),
                "{name} ({mode}): output differs from CPython"
            );
        }
        std::fs::remove_file(&bin).expect("remove the probe binary");
        peaks.push(peak);
    }
    let growth = peaks[1] - peaks[0];
    assert!(
        growth <= GROWTH_LIMIT_KB,
        "{name} ({mode}): peak RSS grew by {growth} kB from {N_SMALL} to {N_LARGE} \
         iterations ({} -> {} kB): an exception leaks every iteration",
        peaks[0],
        peaks[1]
    );
}

macro_rules! probe {
    ($name:ident, $body:expr) => {
        #[test]
        #[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
        fn $name() {
            assert_constant_memory(stringify!($name), "", $body);
        }
    };
}

probe!(
    native_zero_division,
    "try:\n    total += 1 // (i - i)\nexcept ZeroDivisionError:\n    total += 1"
);
probe!(
    caught_with_a_message_binding,
    "try:\n    fail(i + 1)\nexcept ValueError as e:\n    total += len(f\"{e}\")"
);
probe!(
    user_class_caught_by_its_base,
    "try:\n    raise_db(i)\nexcept AppError:\n    total += 1"
);
probe!(
    explicit_cause,
    "try:\n    chained(i + 1)\nexcept AppError:\n    total += 1"
);
probe!(
    bare_reraise,
    "try:\n    reraise(i + 1)\nexcept ValueError as e:\n    total += len(f\"{e}\")"
);
probe!(
    finally_replaces_the_pending_exception,
    "try:\n    finally_replaces(i)\nexcept KeyError as e:\n    total += len(f\"{e}\")"
);
probe!(
    propagates_through_finally,
    "try:\n    through_finally(i + 1)\nexcept ValueError:\n    total += 1"
);
probe!(
    return_from_a_handler,
    "total += len(caught_in_callee(i + 1))"
);
probe!(
    break_and_continue_in_a_handler,
    "total += break_out(3) + continue_on(4)"
);
probe!(
    kept_in_a_list_and_a_slot,
    "xs = [keep_one(i + 1), keep_one(i + 2)]\nholder = Keeper(xs[1])\n\
     total += len(xs) + len(f\"{holder.err}\")"
);
probe!(
    raise_of_an_existing_exception,
    "try:\n    raise keep_one(i + 1)\nexcept ValueError:\n    total += 1"
);
probe!(stop_iteration_value, "total += len(stop_value(i))");
probe!(
    generator_throw,
    "g = counting()\ntotal += next(g)\ntotal += g.throw(ValueError(\"t\" + str(i)))"
);
probe!(
    generator_closed_inside_a_handler,
    "h = suspended_in_handler(i + 1)\ntotal += next(h)"
);
probe!(except_star_partition, "total += groups(i + 1)");
probe!(
    key_error,
    "try:\n    total += D[str(i)]\nexcept KeyError as e:\n    total += len(f\"{e}\")"
);
probe!(
    unwinds_frames_holding_values,
    "try:\n    total += deep(i + 1)\nexcept ValueError:\n    total += 1"
);

/// An exception a CPython call raises is bridged into a native one that
/// carries the CPython original; both are freed once the handler is done.
#[test]
#[ignore = "requires a CPython 3.13+ with a shared libpython (PYCC_PYTHON, default python3)"]
fn cpython_call_raises() {
    assert_constant_memory(
        "cpython_call_raises",
        "import json\n",
        "try:\n    json.loads(\"{bad\")\nexcept ValueError as e:\n    total += len(f\"{e}\")",
    );
}

/// The lifetime fixture (`tests/memory_leaks.rs` runs it natively under
/// memcheck) prints what CPython prints in non-strict mode too.
#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn the_lifetime_fixture_matches_cpython_in_non_strict_mode() {
    let dir = ScratchDir::new("memory_exc_fixture_nonstrict").expect("scratch dir");
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/exception_lifetime.py");
    let source = std::fs::read_to_string(&src).expect("read the fixture");
    let bin = build_with(&dir, "exception_lifetime", &source, &["--non-strict"]);
    let (out, _) = run_measured(&bin);
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(&cpython(&src))
    );
}
