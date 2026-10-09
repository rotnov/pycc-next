//! Part 1 of #1499 (#1501): a module-global `object` slot owns its
//! reference, so rebinding the name releases the previous value exactly
//! once, as CPython does.
//!
//! **The oracle is CPython itself.** Every module below is also run as
//! plain Python source, and the probe expects the compiled extension to
//! report what CPython reports, line for line: the number of live stub
//! objects (each counts itself in `__init__` and `__del__`), the dead
//! weak references, `sys.getrefcount` deltas of mortal stub attributes
//! (PEP 683, see `tests/issue_1084_refcount_probe.rs`), and what a
//! re-entrant finalizer saw. The module body runs its loops at two trip
//! counts, so a leak that grows with the trip count cannot hide behind a
//! constant.
//!
//! **What stays leaked, pinned as such.** A function-local binding keeps
//! #1092's leak-only rule until Part 2 (#1502); the positive control
//! `local_fresh` leaks one object per call, which also shows the probe sees
//! a leak. A value an *earlier* module exec stored is never released (a
//! failed import retried, a re-import after `del sys.modules[name]`), so
//! each such exec leaks at most the one value it left in each slot.
//!
//! The hosted tests are `#[ignore]`d and contribute no line coverage; the
//! Tier-1 `native-build-test` leg runs them with `cargo test --workspace --
//! --include-ignored`. The changed lines are covered by the unit tests in
//! `crates/pycc_codegen/src/object_slot_tests.rs`.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// The foreign stub. Every `Thing` counts itself in `LIVE` and is
/// weak-referenced from `REFS`; a `Peeker`'s finalizer calls `HOOK`, which
/// the host points at the module under test.
const STUB: &str = "\
import array
import importlib
import sys
import threading
import weakref

LIVE = 0
REFS = []
SEEN = []
HOOK = None
NEST = None
NESTED = False
HELD = []


class Thing:
    def __init__(self):
        global LIVE
        LIVE += 1
        REFS.append(weakref.ref(self))

    def __del__(self):
        # A value pycc leaks on purpose (an earlier exec's last value) is
        # finalized at interpreter shutdown, after this module's globals
        # are cleared to `None`; it no longer counts then.
        global LIVE
        if LIVE is not None:
            LIVE -= 1

    def fail(self):
        raise ValueError()


class Peeker(Thing):
    def __del__(self):
        super().__del__()
        if HOOK is not None:
            SEEN.append(HOOK())


def fresh():
    return Thing()


def peeker():
    return Peeker()


def make_things(n):
    return [Thing() for _ in range(n)]


def nest():
    # Drops the module named `NEST` from `sys.modules` and imports it
    # again, once: a second `Py_mod_exec` of the same shared object while
    # the first is suspended in this call.
    global NESTED
    if NEST is None or NESTED:
        return None
    NESTED = True
    outer = sys.modules[NEST]
    del sys.modules[NEST]
    try:
        importlib.import_module(NEST)
    finally:
        sys.modules[NEST] = outer
    return None


def hold(o, _):
    # Reads `o`, a borrowed global the caller kept across `nest()`.
    HELD.append(type(o).__name__)


PAUSED = threading.Event()
GO = threading.Event()
DONE = threading.Event()


def pause():
    # Suspends the module body, the GIL released, until the host lets it go.
    PAUSED.set()
    GO.wait(30)


class Exporter:
    # A float64 buffer whose release hands the GIL to the paused body and
    # waits until that body has finished.
    def __buffer__(self, flags):
        return memoryview(array.array('d', [1.0]))

    def __release_buffer__(self, view):
        GO.set()
        DONE.wait(30)
        view.release()


T = Thing()
PAIR = (Thing(), Thing())
";

/// The module under test, with `{N}` the trip count. Each shape binds a
/// module global: a rebind loop of produced values, a produced attribute
/// load, an alias of a borrowed global, `x = x`, a borrowed global stored
/// into a compiled-instance attribute and then rebound, a compiled
/// instance rebound, a tuple unpack, a `for` target over the fixed
/// `PAIR`, and an empty-body `for` over `{N}` fresh items, whose target
/// keeps only the last one alive. The first rebind
/// releases a `Peeker`, whose finalizer reads the global back through
/// `peek()` and must see the new value.
const MODULE: &str = "\
import pycc_t1499_stub as s


class Box:
    def __init__(self, a: object) -> None:
        self.a = a


def peek() -> object:
    return x


def get_x() -> object:
    return x


def get_kept() -> object:
    return box.a


def get_same() -> object:
    return same


def get_inst() -> object:
    return inst


def local_fresh() -> None:
    z = s.fresh()


x: object = s.peeker()
x = s.fresh()
for i in range({N}):
    x = s.fresh()
t: object = None
for i in range({N}):
    t = s.T
alias: object = None
for i in range({N}):
    alias = s.fresh()
    x = alias
same: object = s.fresh()
for i in range({N}):
    same = same
box = Box(x)
x = s.fresh()
inst: object = None
for i in range({N}):
    inst = Box(None)
p: object = None
q: object = None
for i in range({N}):
    p, q = s.PAIR
for w in s.PAIR:
    p = w
for e in s.make_things({N}):
    pass
";

/// Writes the stub into the scratch root, every `(module, source)` as
/// `src/<module>.py` built into the scratch root, and the same source as
/// plain Python `pure/<module>_py.py`, the CPython oracle.
fn build(tag: &str, modules: &[(&str, &str)]) -> ScratchDir {
    let dir = ScratchDir::new(tag).expect("scratch");
    std::fs::write(dir.join("pycc_t1499_stub.py"), STUB).expect("write the stub");
    std::fs::create_dir_all(dir.join("src")).expect("create the entry directory");
    std::fs::create_dir_all(dir.join("pure")).expect("create the oracle directory");
    for (module, source) in modules {
        let entry = dir.join("src").join(format!("{module}.py"));
        std::fs::write(&entry, source).expect("write the module");
        std::fs::write(dir.join("pure").join(format!("{module}_py.py")), source)
            .expect("write the oracle");
        let build = pycc()
            .arg("build")
            .arg(&entry)
            .arg("-o")
            .arg(dir.join(module))
            .arg("--ext")
            .output()
            .expect("pycc should spawn");
        assert!(build.status.success(), "{}", stderr_of(&build));
    }
    dir
}

/// Runs `script` with `module` as `sys.argv[1]` under the host interpreter,
/// with `flags` before `-c` and `env` added, and returns what it reported on
/// stderr.
fn python(dir: &Path, flags: &[&str], env: &[(&str, &str)], module: &str, script: &str) -> String {
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .args(flags)
        .arg("-c")
        .arg(script)
        .arg(module)
        .envs(env.iter().copied())
        .current_dir(dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(stdout_of(&run), "");
    stderr_of(&run)
}

/// A host prelude: the stub, `report(*fields)`, and `dead()`, the number of
/// stub objects whose weak reference is dead.
const PRELUDE: &str = "\
import gc, importlib, sys
sys.path.insert(0, '.')
sys.path.insert(0, 'pure')
import pycc_t1499_stub as s
name = sys.argv[1]
def report(*fields):
    print(*fields, file=sys.stderr, flush=True)
def dead():
    return sum(1 for r in s.REFS if r() is None)
";

/// The bindings probe: imports `sys.argv[1]` and reports every count.
fn bindings_script() -> String {
    format!(
        "{PRELUDE}\
         def hook():\n\
         \x20   m = sys.modules.get(name)\n\
         \x20   return type(m.peek()).__name__ if m is not None else 'unbound'\n\
         s.HOOK = hook\n\
         live, t = s.LIVE, sys.getrefcount(s.T)\n\
         pair = [sys.getrefcount(o) for o in s.PAIR]\n\
         m = importlib.import_module(name)\n\
         gc.collect()\n\
         report('live', s.LIVE - live, 'dead', dead())\n\
         report('seen', *s.SEEN)\n\
         report('T', sys.getrefcount(s.T) - t)\n\
         report('PAIR', *[sys.getrefcount(o) - b for o, b in zip(s.PAIR, pair)])\n\
         report('kept', type(m.get_kept()).__name__, m.get_kept() is m.get_x())\n\
         report('same', sys.getrefcount(m.get_same()))\n\
         report('inst', type(m.get_inst()).__name__, sys.getrefcount(m.get_inst()))\n"
    )
}

/// What CPython reports for `MODULE` at `trips`: `x`, `same`, the
/// object `box.a` keeps and the `for` target `e`'s last item are alive, and
/// every other `Thing` the body made -- the `Peeker`, each rebound value
/// and every earlier `for` item -- is dead.
fn expected_bindings(trips: usize) -> String {
    format!(
        "live 4 dead {}\nseen Thing\nT 1\nPAIR 1 4\nkept Thing False\nsame 2\ninst Box 2\n",
        3 * trips
    )
}

/// The module-level bindings release every rebound value exactly once, at
/// two trip counts, exactly as CPython does; a function-local binding
/// still leaks one object per call (Part 2, #1502).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_module_global_rebind_releases_the_previous_value_as_cpython_does() {
    let modules: Vec<(String, String)> = [50, 150]
        .iter()
        .map(|n| {
            (
                format!("pycc_t1499_m{n}"),
                MODULE.replace("{N}", &n.to_string()),
            )
        })
        .collect();
    let refs: Vec<(&str, &str)> = modules
        .iter()
        .map(|(m, s)| (m.as_str(), s.as_str()))
        .collect();
    let dir = build("t1499_bindings", &refs);
    let script = bindings_script();
    for (trips, (module, _)) in [50, 150].into_iter().zip(&modules) {
        let compiled = python(&dir, &[], &[], module, &script);
        let oracle = python(&dir, &[], &[], &format!("{module}_py"), &script);
        assert_eq!(compiled, oracle, "{module} against CPython");
        assert_eq!(compiled, expected_bindings(trips), "{module}");
    }
    let leak = format!(
        "{PRELUDE}\
         m = importlib.import_module(name)\n\
         live = s.LIVE\n\
         for _ in range(50):\n\
         \x20   m.local_fresh()\n\
         report('local', s.LIVE - live)\n"
    );
    let module = &modules[0].0;
    assert_eq!(python(&dir, &[], &[], module, &leak), "local 50\n");
    assert_eq!(
        python(&dir, &[], &[], &format!("{module}_py"), &leak),
        "local 0\n"
    );
}

/// The same probe under CPython's development mode and debug allocator,
/// which detect a release of freed memory and a refcount underflow.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_rebind_probe_is_clean_under_the_debug_allocator() {
    let source = MODULE.replace("{N}", "50");
    let dir = build("t1499_dev", &[("pycc_t1499_dev", source.as_str())]);
    let got = python(
        &dir,
        &["-X", "dev"],
        &[("PYTHONMALLOC", "debug")],
        "pycc_t1499_dev",
        &bindings_script(),
    );
    assert_eq!(got, expected_bindings(50));
}

/// Import bindings own their reference too: a from-import of the same name
/// twice, top-level or nested in a module-level `if` (#1383), holds one
/// reference, as CPython does.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_repeated_from_import_holds_one_reference() {
    let dir = build(
        "t1499_from",
        &[
            (
                "pycc_t1499_from",
                "from pycc_t1499_stub import T\nfrom pycc_t1499_stub import T\n",
            ),
            (
                "pycc_t1499_nested",
                "import pycc_t1499_stub as s\n\n\
                 if s.LIVE >= 0:\n    \
                 from pycc_t1499_stub import T\n    \
                 from pycc_t1499_stub import T\n",
            ),
        ],
    );
    let script = format!(
        "{PRELUDE}\
         t = sys.getrefcount(s.T)\n\
         importlib.import_module(name)\n\
         report('T', sys.getrefcount(s.T) - t)\n"
    );
    for module in ["pycc_t1499_from", "pycc_t1499_nested"] {
        assert_eq!(python(&dir, &[], &[], module, &script), "T 1\n", "{module}");
        assert_eq!(
            python(&dir, &[], &[], &format!("{module}_py"), &script),
            "T 1\n",
            "{module} under CPython"
        );
    }
}

/// The module under test for the nested exec: the outer body keeps its
/// `x` borrowed as an argument across `s.nest()`, which runs the same body
/// again beneath it. The nested body's own `hold` finds nothing to nest.
const NESTED_MODULE: &str = "\
import pycc_t1499_stub as s


def get_x() -> object:
    return x


x: object = s.fresh()
x = s.fresh()
s.hold(x, s.nest())
x = s.fresh()
x = s.fresh()
";

/// #1501: a rebind releases the replaced value only while the rebinding
/// exec is the artifact's only live compiled activation. The nested exec
/// runs beneath the outer one, so each of its rebinds leaks the value it
/// replaces instead of releasing it, and both `hold` calls read a live
/// object -- as under CPython, whose report the compiled one matches
/// except for the counts. The outer exec, alone again after the nested
/// one returns, still releases what it rebinds: the positive control.
///
/// Compiled, the outer body makes `a1`, `a2` (releasing `a1`), then the
/// nested body `b1`..`b4`: the bits it clears at its entry leave `a2`
/// unreleased, and the gate leaves `b1` and `b2` unreleased too, where
/// releasing them would have been sound (no frame holds them). Its own
/// `hold` reads `b2`, still live. Back in the outer body, `hold` reads
/// `a2`, and its rebinds to `a3` and `a4` release `b4` and `a3`. Alive:
/// `a2`, `b1`, `b2`, `b3`, `a4`; dead: `a1`, `b4`, `a3`. CPython gives each
/// module its own `x`, and the nested module, dropped once `nest` restores
/// the outer one, is collected with its `b4`: alive `a4`, dead the rest.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_rebind_beneath_another_live_exec_leaks_instead_of_releasing() {
    let dir = build("t1499_nested", &[("pycc_t1499_nested_exec", NESTED_MODULE)]);
    let script = format!(
        "{PRELUDE}\
         s.NEST = name\n\
         live = s.LIVE\n\
         m = importlib.import_module(name)\n\
         gc.collect()\n\
         report('held', *s.HELD)\n\
         report('x', type(m.get_x()).__name__)\n\
         report('live', s.LIVE - live, 'dead', dead())\n"
    );
    let flags: &[&str] = &["-X", "dev"];
    let env = &[("PYTHONMALLOC", "debug")];
    let compiled = python(&dir, flags, env, "pycc_t1499_nested_exec", &script);
    let oracle = python(&dir, flags, env, "pycc_t1499_nested_exec_py", &script);
    assert_eq!(compiled, "held Thing Thing\nx Thing\nlive 5 dead 3\n");
    assert_eq!(oracle, "held Thing Thing\nx Thing\nlive 1 dead 7\n");
}

/// The module under test for a wrapper-entered activation: the body only
/// binds `x`; the host later calls the export `probe`, which keeps `x`
/// borrowed as an argument across `s.nest()`, so the same body runs again
/// beneath a live wrapper frame rather than beneath another exec.
const WRAPPED_MODULE: &str = "\
import pycc_t1499_stub as s


def get_x() -> object:
    return x


def probe() -> None:
    s.hold(x, s.nest())


x: object = s.fresh()
x = s.fresh()
x = s.fresh()
";

/// #1501: a compiled call a host entered through a wrapper is a live
/// activation too. The outer exec has returned, so `probe`'s wrapper is the
/// only other activation while the nested exec rebinds; the nested rebinds
/// leak, and `hold` reads a live object. The outer exec, which ran alone,
/// released what it rebound: the positive control.
///
/// Compiled, the outer body makes `a1`..`a3`, releasing `a1` and `a2`.
/// `probe` holds `a3` across the nested body, which makes `b1`..`b3`: the
/// bit it clears at its entry leaves `a3` unreleased, and the gate leaves
/// `b1` and `b2` unreleased. Alive: `a3`, `b1`, `b2`, `b3`; dead: `a1`,
/// `a2`. Were the wrapper frame not counted, the nested exec would see
/// itself alone and release `b1` and `b2` (`live 2 dead 4`). CPython's
/// nested module is collected with its `b3`: alive `a3`, dead the rest.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_rebind_beneath_a_live_wrapper_call_leaks_instead_of_releasing() {
    let dir = build(
        "t1499_wrapped",
        &[("pycc_t1499_wrapped_exec", WRAPPED_MODULE)],
    );
    let script = format!(
        "{PRELUDE}\
         s.NEST = name\n\
         live = s.LIVE\n\
         m = importlib.import_module(name)\n\
         gc.collect()\n\
         report('after import', s.LIVE - live, 'dead', dead())\n\
         m.probe()\n\
         gc.collect()\n\
         report('held', *s.HELD)\n\
         report('x', type(m.get_x()).__name__)\n\
         report('live', s.LIVE - live, 'dead', dead())\n"
    );
    let flags: &[&str] = &["-X", "dev"];
    let env = &[("PYTHONMALLOC", "debug")];
    let compiled = python(&dir, flags, env, "pycc_t1499_wrapped_exec", &script);
    let oracle = python(&dir, flags, env, "pycc_t1499_wrapped_exec_py", &script);
    assert_eq!(
        compiled,
        "after import 1 dead 2\nheld Thing\nx Thing\nlive 4 dead 2\n"
    );
    assert_eq!(
        oracle,
        "after import 1 dead 2\nheld Thing\nx Thing\nlive 1 dead 5\n"
    );
}

/// A module exec that ran before -- a failed import retried, a re-import
/// after `del sys.modules[name]` -- leaves its last value in each slot
/// unreleased, so each such exec leaks at most one object per slot; the
/// values one exec rebinds are still released. `importlib.reload` does not
/// run the exec again, and an earlier module's function stays usable.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_earlier_exec_leaks_at_most_its_last_value() {
    let dir = build(
        "t1499_reexec",
        &[
            (
                "pycc_t1499_fail",
                "import pycc_t1499_stub as s\n\n\
                 x: object = s.fresh()\nx = s.fresh()\ns.T.fail()\n",
            ),
            (
                "pycc_t1499_again",
                "import pycc_t1499_stub as s\n\n\n\
                 def get_x() -> object:\n    return x\n\n\n\
                 x: object = s.fresh()\nx = s.fresh()\n",
            ),
        ],
    );
    let retried = format!(
        "{PRELUDE}\
         live, caught = s.LIVE, 0\n\
         for _ in range(50):\n\
         \x20   try:\n\
         \x20       importlib.import_module(name)\n\
         \x20   except ValueError:\n\
         \x20       caught += 1\n\
         \x20   sys.modules.pop(name, None)\n\
         gc.collect()\n\
         report('retried', caught, s.LIVE - live, dead())\n"
    );
    assert_eq!(
        python(&dir, &[], &[], "pycc_t1499_fail", &retried),
        "retried 50 50 50\n"
    );
    assert_eq!(
        python(&dir, &[], &[], "pycc_t1499_fail_py", &retried),
        "retried 50 0 100\n"
    );
    let again = format!(
        "{PRELUDE}\
         live = s.LIVE\n\
         first = importlib.import_module(name)\n\
         get_x = first.get_x\n\
         for _ in range(3):\n\
         \x20   importlib.reload(first)\n\
         report('reload', s.LIVE - live, type(get_x()).__name__)\n\
         for _ in range(3):\n\
         \x20   del sys.modules[name]\n\
         \x20   importlib.import_module(name)\n\
         gc.collect()\n\
         report('again', s.LIVE - live, type(get_x()).__name__)\n"
    );
    assert_eq!(
        python(&dir, &[], &[], "pycc_t1499_again", &again),
        "reload 1 Thing\nagain 4 Thing\n"
    );
}

/// The module under test for a wrapper suspended in its own cleanup: the
/// body binds `x` twice, then pauses in `s.pause()` (the GIL released)
/// until the host lets it go, and rebinds `x` once more.
const SUSPENDED_MODULE: &str = "\
import pycc_t1499_stub as s


def get_x(b: memoryview) -> object:
    return x


x: object = s.fresh()
x = s.fresh()
s.pause()
x = s.fresh()
";

/// The second Codex P1 on #1505: a wrapper stays a live activation until
/// its return value holds its own host reference. A thread imports the
/// module, whose body pauses after binding `x` (`a2`); the host calls
/// `get_x` with an `Exporter`, whose `__release_buffer__` -- run by the
/// wrapper's `PyBuffer_Release`, after the compiled call returned `a2`
/// borrowed and before the pack -- lets the body go and waits until it has
/// rebound `x` to `a3` and returned. Were the wrapper counted out before
/// that cleanup, the paused exec would see itself alone, release `a2`, and
/// the pack would take a reference to freed memory, which the debug
/// allocator catches. Counted, the rebind leaks `a2` (one reference more
/// than under CPython) and the host gets a live `a2`.
///
/// CPython calls no buffer hook for a pure function's argument, so the
/// host lets the body go itself after the call; there `got` is `a2` too,
/// read before the rebind. Alive in both: `a2` and `a3`; dead: `a1`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_wrapper_suspended_in_its_cleanup_still_counts_as_a_live_activation() {
    let dir = build(
        "t1499_suspended",
        &[("pycc_t1499_suspended", SUSPENDED_MODULE)],
    );
    let script = format!(
        "{PRELUDE}\
         import threading\n\
         live = s.LIVE\n\
         def run():\n\
         \x20   importlib.import_module(name)\n\
         \x20   s.DONE.set()\n\
         th = threading.Thread(target=run)\n\
         th.start()\n\
         s.PAUSED.wait(30)\n\
         got = sys.modules[name].get_x(s.Exporter())\n\
         s.GO.set()\n\
         th.join()\n\
         m = sys.modules[name]\n\
         gc.collect()\n\
         report('got', type(got).__name__, got is m.get_x(s.Exporter()))\n\
         report('refs', sys.getrefcount(got))\n\
         report('live', s.LIVE - live, 'dead', dead())\n"
    );
    let flags: &[&str] = &["-X", "dev"];
    let env = &[("PYTHONMALLOC", "debug")];
    let compiled = python(&dir, flags, env, "pycc_t1499_suspended", &script);
    let oracle = python(&dir, flags, env, "pycc_t1499_suspended_py", &script);
    assert_eq!(compiled, "got Thing False\nrefs 3\nlive 2 dead 1\n");
    assert_eq!(oracle, "got Thing False\nrefs 2\nlive 2 dead 1\n");
}
