//! Native memory management across the `--ext` boundary: a CPython host
//! calling a compiled module in a loop leaves the runtime's live-object
//! counters flat. Split out of `tests/memory_leaks.rs`.

use pycc_scratch::ScratchDir;
use std::process::Command;

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
/// the runtime's live list, instance, exception and `str` counts flat.
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
counters = [
    counter('pycc_rt_%s_live_objects' % k) for k in ('list', 'instance', 'exception', 'str')
]
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
assert after == before, 'live list/instance/exception/str counts grew: %r -> %r' % (
    before, after)
print('ok')
"#;

/// The generator `throw()` paths -- a thrown exception caught inside the
/// generator, one propagating out of it, and either thrown through a
/// `yield from` delegation -- release the generators and the counted values
/// their frames hold, and the exceptions thrown: counted through the
/// runtime's own counters rather than peak memory.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn generator_throw_paths_release_their_frames() {
    let dir = ScratchDir::new("memory_ext_generator_throw").expect("scratch dir");
    let src = dir.join("pycc_memory_gen.py");
    std::fs::write(&src, GENERATOR_THROW_MODULE).expect("write the module source");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_memory_gen"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    std::fs::remove_file(&src).expect("remove the source");
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(GENERATOR_THROW_DRIVER)
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

const GENERATOR_THROW_MODULE: &str = r#"from typing import Generator


def _mk(i: int) -> list[int]:
    return [i, i]


def _catcher(i: int) -> Generator[int, int, int]:
    xs = _mk(i)
    total = 0
    try:
        while True:
            try:
                got = yield len(xs)
                total += got
            except ValueError:
                xs = _mk(i + 1)
                total += 100
            if total > 1000:
                return total
    finally:
        xs = _mk(0)


def _outer(i: int) -> Generator[int, int, int]:
    held = _mk(i)
    result = yield from _catcher(i)
    return result + len(held)


def throw_caught(i: int) -> int:
    g = _catcher(i)
    n = next(g)
    n += g.send(3)
    n += g.throw(ValueError("x"))
    g.close()
    return n


def throw_uncaught(i: int) -> int:
    g = _catcher(i)
    next(g)
    try:
        g.throw(KeyError("k"))
    except KeyError:
        return 1
    return 0


def delegated(i: int) -> int:
    g = _outer(i)
    n = next(g)
    n += g.send(5)
    n += g.throw(ValueError("y"))
    try:
        g.send(2000)
    except StopIteration as e:
        n += e.value
    return n


def delegated_close(i: int) -> int:
    g = _outer(i)
    n = next(g)
    g.close()
    return n
"#;

/// The values are CPython's for the same module.
const GENERATOR_THROW_DRIVER: &str = r#"import ctypes, gc
import pycc_memory_gen as m
assert m.__file__.endswith('.so')
lib = ctypes.CDLL(m.__file__)
def counter(name):
    f = getattr(lib, name)
    f.restype = ctypes.c_int64
    f.argtypes = []
    return f
counters = [counter('pycc_rt_%s_live_objects' % k) for k in ('list', 'gen', 'exception')]
def live():
    gc.collect()
    return [c() for c in counters]
def round_trip():
    assert m.throw_caught(1) == 6
    assert m.throw_uncaught(1) == 1
    assert m.delegated(1) == 2113
    assert m.delegated_close(1) == 2
round_trip()
before = live()
for _ in range(200):
    round_trip()
after = live()
assert after == before, 'live list/generator/exception counts grew: %r -> %r' % (before, after)
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

/// Exceptions crossing the `--ext` boundary are freed: one a compiled
/// function raises to the host, one it raises and catches itself, and one a
/// host callable raises into compiled code -- caught there (the bridged
/// CPython original dies with its handler) or escaping unchanged (the host
/// gets the very object it raised). The runtime's live exception and `str`
/// counts stay flat, and no host exception outlives the loop.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn exceptions_crossing_the_boundary_are_freed() {
    let dir = ScratchDir::new("memory_ext_exceptions").expect("scratch dir");
    let src = dir.join("m.py");
    std::fs::write(&src, EXCEPTION_MODULE).expect("write the module source");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_memory_exc"))
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
        .arg(EXCEPTION_DRIVER)
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

const EXCEPTION_MODULE: &str = r#"def raise_native(n: int) -> int:
    if n >= 0:
        raise ValueError("native " + str(n))
    return n


def catch_native(n: int) -> int:
    total = 0
    for k in range(n):
        try:
            raise_native(k)
        except ValueError as e:
            total += len(f"{e}")
    return total


def catch_foreign(f: object) -> int:
    try:
        f()
    except ValueError as e:
        return len(f"{e}")
    return 0


def through_foreign(f: object) -> int:
    f()
    return 0
"#;

const EXCEPTION_DRIVER: &str = r#"import ctypes, gc, weakref
import pycc_memory_exc as m
lib = ctypes.CDLL(m.__file__)
def counter(name):
    f = getattr(lib, name)
    f.restype = ctypes.c_int64
    f.argtypes = []
    return f
counters = [counter('pycc_rt_%s_live_objects' % k) for k in ('exception', 'str')]
def live():
    gc.collect()
    return [c() for c in counters]
class Boom(ValueError):
    pass
raised = weakref.WeakSet()
def thrower():
    b = Boom('boom')
    raised.add(b)
    raise b
def round_trip():
    try:
        m.raise_native(3)
    except ValueError as e:
        assert str(e) == 'native 3', e
    else:
        raise AssertionError('raise_native')
    assert m.catch_native(4) == 32
    assert m.catch_foreign(thrower) == 4
    try:
        m.through_foreign(thrower)
    except Boom as e:
        assert e in raised
    else:
        raise AssertionError('through_foreign')
round_trip()
before = live()
for _ in range(300):
    round_trip()
after = live()
assert after == before, 'live exception/str counts grew: %r -> %r' % (before, after)
gc.collect()
assert len(raised) == 0, '%d host exceptions outlived the loop' % len(raised)
print('ok')
"#;
