//! Part 5 of #1026: what the foreign-object loop shape costs in references.
//!
//! `tests/issue_1084_loop_shape.rs` shows the shape computes the right
//! answer. This file measures the price: across a loop of `N` trips, the
//! `sys.getrefcount` of a foreign module attribute must not move at all,
//! however many times a trip reads the attribute out of the foreign module
//! (`PRODUCING_OPERATIONS`) and feeds it to a consuming conversion --
//! `float`, `bool`, `int`, `str`, `len`, or the fixed-arity tuple unpack.
//!
//! Until Part 1 of #1092 (the release protocol for CPython object
//! temporaries, `crates/pycc_codegen/src/object_release.rs`) the attribute
//! load feeding each conversion leaked one reference, so these deltas were
//! pinned at `producing_operations * N`. Each conversion now releases the
//! temporary it consumed, and the pin is `0` at both trip counts: a leak
//! that came back would show up as a slope, and an over-release as a
//! negative delta (or a crash once the attribute's count reaches zero).
//!
//! **PEP 683 is why the subject looks the way it does.** From CPython 3.12,
//! small integers, `None`, `True`/`False` and interned strings are immortal:
//! `sys.getrefcount(7)` is `4294967295` and never moves. A probe whose stub
//! exported `COUNT = 7` would measure `+0` for every shape at every trip
//! count and prove nothing at all, passing just as happily against a runtime
//! that leaked a reference on every single operation. Every attribute the
//! stub below exports is therefore a mortal object: a `float`, an `int` far
//! outside the small-integer cache, a `tuple`, and an instance of a
//! module-local class.
//!
//! The stub exports more protocol than the probe below reads -- `MESH`
//! carries `__getitem__`, `__iter__` and a `sample` method that no arm here
//! calls -- so that extending the probe to another producing shape needs a
//! change to the loop, not to the fixture. `sample` avoids a
//! container-protocol name deliberately: a container-named method on a
//! foreign object goes through #1095's receiver dispatch, and an arm added
//! under such a name would measure that seam instead of this one.
//!
//! The hosted tests contribute no line coverage (CI's coverage job runs
//! `llvm-cov` without `--include-ignored`) and add no Rust lines outside
//! `tests/`; they are run by the Tier-1 `native-build-test` leg's
//! `cargo test --workspace -- --include-ignored`.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

/// Normalizes the line-ending convention CPython's text layer uses on
/// Windows, so the captured-output assertions below hold on every Tier-1
/// platform.
fn normalize_newlines(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn stdout_of(output: &Output) -> String {
    normalize_newlines(&output.stdout)
}

fn stderr_of(output: &Output) -> String {
    normalize_newlines(&output.stderr)
}

/// Every exported attribute is mortal, for the PEP 683 reason in this file's
/// header. `BIGCOUNT` is `10 ** 9 + 7`, far past the small-integer cache but
/// still inside the D-141 inline-integer range `int(o)` accepts.
const STUB: &str = "\
SCALE = 2.5
BIGCOUNT = 10 ** 9 + 7
POINT = (1.5, 2.5, 3.5)


class _Scale:
    def __len__(self):
        return 3

    def __getitem__(self, i):
        return SCALE

    def __iter__(self):
        return iter((SCALE, SCALE, SCALE))

    def sample(self, i: int):
        return SCALE


MESH = _Scale()
";

/// The attributes the probe reads, paired with how many times one trip of the
/// loop below reads each of them out of the foreign module.
const PRODUCING_OPERATIONS: &[(&str, u32)] = &[
    // `float(SCALE)` and `bool(SCALE)`.
    ("SCALE", 2),
    // `int(BIGCOUNT)` and `str(BIGCOUNT)`.
    ("BIGCOUNT", 2),
    // `len(MESH)`.
    ("MESH", 1),
    // The fixed-arity all-`float` tuple unpack.
    ("POINT", 1),
];

/// One `for` loop over `trips` trips, touching every Part 4 conversion once
/// per trip. Generated from a single template so the operation counts in
/// `PRODUCING_OPERATIONS` cannot drift between the two trip counts.
fn probe_source(trips: u32) -> String {
    format!(
        "import pycc_p5_probe\n\
         \n\
         f: float = 0.0\n\
         n: int = 0\n\
         b: bool = False\n\
         s: str = \"\"\n\
         m: int = 0\n\
         x: float = 0.0\n\
         for i in range({trips}):\n\
         \x20   f = f + float(pycc_p5_probe.SCALE)\n\
         \x20   b = bool(pycc_p5_probe.SCALE)\n\
         \x20   n = n + int(pycc_p5_probe.BIGCOUNT)\n\
         \x20   s = str(pycc_p5_probe.BIGCOUNT)\n\
         \x20   m = m + len(pycc_p5_probe.MESH)\n\
         \x20   p: tuple[float, float, float] = pycc_p5_probe.POINT\n\
         \x20   x = x + p[0]\n\
         \n\
         \n\
         def total() -> float:\n\
         \x20   return f\n"
    )
}

/// Writes the stub into the scratch root and the named entry module into
/// `src/` beneath it. `tests/issue_1084_loop_shape.rs` explains why the stub
/// must not sit beside the entry module.
fn fixture(category: &str, module: &str, source: &str) -> ScratchDir {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("pycc_p5_probe.py"), STUB).expect("write the foreign stub");
    std::fs::create_dir_all(dir.join("src")).expect("create the entry directory");
    std::fs::write(dir.join("src").join(format!("{module}.py")), source).expect("write the probe");
    dir
}

/// Builds `module` as an extension module directly into `dir`.
///
/// The output path carries no extension suffix; see
/// `tests/issue_1084_loop_shape.rs`'s own `build_ext` for why spelling one
/// breaks the `--ext` module-name contract on Windows.
fn build_ext(dir: &Path, module: &str) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("src").join(format!("{module}.py")))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

fn python(dir: &Path, script: &str) -> Output {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

/// Builds a probe at `trips` trips, imports it in its **own** interpreter
/// process, and returns the measured `sys.getrefcount` delta for each
/// attribute in `PRODUCING_OPERATIONS`.
///
/// Its own process, because importing two probes into one interpreter would
/// make the second probe's baseline the first probe's endpoint -- still
/// arithmetically correct, but needlessly easy to misread.
fn refcount_deltas(category: &str, module: &str, trips: u32) -> Vec<(String, i64)> {
    let dir = fixture(category, module, &probe_source(trips));
    let build = build_ext(&dir, module);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let names: Vec<&str> = PRODUCING_OPERATIONS.iter().map(|(name, _)| *name).collect();
    let script = format!(
        "import sys\n\
         sys.path.insert(0, '.')\n\
         import pycc_p5_probe as stub\n\
         names = {names:?}\n\
         before = {{k: sys.getrefcount(getattr(stub, k)) for k in names}}\n\
         import {module}\n\
         after = {{k: sys.getrefcount(getattr(stub, k)) for k in names}}\n\
         for k in names:\n\
         \x20   print(k, after[k] - before[k])\n"
    );
    let run = python(&dir, &script);
    assert!(run.status.success(), "{}", stderr_of(&run));

    stdout_of(&run)
        .lines()
        .map(|line| {
            let (name, delta) = line.split_once(' ').expect("probe prints `name delta`");
            (
                name.to_string(),
                delta.parse::<i64>().expect("probe prints an integer delta"),
            )
        })
        .collect()
}

/// The invariant, at two trip counts: every attribute's delta is exactly
/// `0`, whatever its number of producing operations.
///
/// Two trip counts rather than one because a single count cannot distinguish
/// a per-trip error from a constant one. The pair pins the slope at zero.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn no_producing_operation_leaks_a_reference() {
    for (category, module, trips) in [
        ("p5_refcount_n1000", "probe_n1000", 1000_u32),
        ("p5_refcount_n2000", "probe_n2000", 2000_u32),
    ] {
        let measured = refcount_deltas(category, module, trips);
        let expected: Vec<(String, i64)> = PRODUCING_OPERATIONS
            .iter()
            .map(|(name, count)| {
                assert!(*count > 0, "{name} is read at least once per trip");
                ((*name).to_string(), 0)
            })
            .collect();
        assert_eq!(
            measured, expected,
            "the {trips}-trip probe moved a foreign attribute's reference count"
        );
    }
}

/// A lone producing operation balances on its own too: a loop that only
/// calls `len(MESH)` leaves `MESH`'s reference count where it found it, and
/// still computes the right total.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_lone_producing_operation_leaks_nothing() {
    const TRIPS: u32 = 1000;
    let source = format!(
        "import pycc_p5_probe\n\
         \n\
         m: int = 0\n\
         for i in range({TRIPS}):\n\
         \x20   m = m + len(pycc_p5_probe.MESH)\n\
         \n\
         \n\
         def total() -> int:\n\
         \x20   return m\n"
    );
    let dir = fixture("p5_refcount_lone", "probe_lone", &source);
    let build = build_ext(&dir, "probe_lone");
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = python(
        &dir,
        "import sys\n\
         sys.path.insert(0, '.')\n\
         import pycc_p5_probe as stub\n\
         before = sys.getrefcount(stub.MESH)\n\
         import probe_lone\n\
         print(sys.getrefcount(stub.MESH) - before, probe_lone.total())\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), format!("0 {}\n", TRIPS * 3));
}

/// The pycc-side instrument. The refcount arm above sees only CPython's
/// reference counts; it is blind to anything pycc allocates on its own heap
/// and never retires. Peak RSS at two trip counts, two orders of magnitude
/// apart, covers that blind spot.
///
/// `#[cfg(unix)]` gates the whole module, not just the assertion: `libc` is a
/// `cfg(unix)` dev-dependency, so this would not compile on `windows-latest`,
/// which is Tier-1 and runs the suite with `--include-ignored`. Follows
/// `tests/issue_146_bigint_release.rs`'s `peak_rss` module.
#[cfg(unix)]
mod peak_rss {
    use super::{PRODUCING_OPERATIONS, build_ext, fixture, stderr_of};
    use std::process::Command;

    /// Spawns `command`, waits for it via `wait4`, and returns the child's
    /// `ru_maxrss`. Duplicated from `tests/issue_146_bigint_release.rs` per
    /// this repository's convention that each integration test carries its
    /// own helpers.
    #[allow(clippy::zombie_processes)]
    fn peak_rss(mut command: Command) -> libc::c_long {
        let child = command.spawn().expect("command must spawn");
        let pid = child.id() as libc::pid_t;
        let mut status = 0;
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        let waited = loop {
            // SAFETY: `pid` identifies the live child spawned immediately
            // above; both output pointers are valid for writes for the
            // duration of the call, and a successful `wait4` initializes the
            // complete `rusage` value.
            let result = unsafe { libc::wait4(pid, &mut status, 0, usage.as_mut_ptr()) };
            if result != -1 {
                break result;
            }
            let error = std::io::Error::last_os_error();
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::Interrupted,
                "wait failed for {command:?}: {error}"
            );
        };
        assert_eq!(waited, pid, "wait4 reaped the wrong child for {command:?}");
        // SAFETY: the successful `wait4` above initialized `usage`.
        let usage = unsafe { usage.assume_init() };
        assert!(
            libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
            "command failed: {command:?}"
        );
        // `ru_maxrss` is the peak resident set, in bytes on macOS/BSD and in
        // kilobytes on Linux -- which is why the assertion below is a ratio
        // of two readings from the same platform, never an absolute.
        usage.ru_maxrss
    }

    /// Builds a `str(o)` loop at `trips` trips and returns the peak RSS of
    /// the *host interpreter process* that imports it. The build itself is
    /// not measured: `pycc`'s own footprint is dominated by LLVM and would
    /// swamp the signal.
    ///
    /// The `str` result is **bound and reassigned**, never discarded. A
    /// discarded `str(o)` temporary is #1109 -- it leaks a pycc-side
    /// `PyStrObj` of roughly 97 bytes per trip, which at 200000 trips would
    /// be about 19 MB against an interpreter floor near 15 MB and would fail
    /// the ratio below. That is a different open issue, and measuring it here
    /// would make this arm a second #1109 regression test instead of the
    /// blind-spot check it is.
    fn probe_peak_rss(category: &str, module: &str, trips: u32) -> libc::c_long {
        let source = format!(
            "import pycc_p5_probe\n\
             \n\
             s: str = \"\"\n\
             for i in range({trips}):\n\
             \x20   s = str(pycc_p5_probe.BIGCOUNT)\n\
             \n\
             \n\
             def out() -> str:\n\
             \x20   return s\n"
        );
        let dir = fixture(category, module, &source);
        let build = build_ext(&dir, module);
        assert!(build.status.success(), "{}", stderr_of(&build));

        let mut command =
            Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()));
        command
            .arg("-c")
            .arg(format!("import {module}"))
            .current_dir(&*dir);
        peak_rss(command)
    }

    /// A hundredfold increase in trips must not move the peak resident set.
    ///
    /// The bound is generous rather than tight, and deliberately so: the
    /// measurement floor is a whole CPython interpreter (~15 MB), it is
    /// noisy, and two readings of the same program routinely differ by a few
    /// percent in either direction. It still has real teeth -- a per-trip
    /// pycc-side allocation of even ~100 bytes would add ~19 MB at 200000
    /// trips, more than doubling the floor.
    #[test]
    #[ignore = "requires a CPython 3.13+ with development headers on PATH"]
    fn the_pycc_heap_does_not_grow_with_the_trip_count() {
        // Referenced so the two arms of this file cannot drift apart on the
        // attribute name the probe reads.
        assert!(
            PRODUCING_OPERATIONS
                .iter()
                .any(|(name, _)| *name == "BIGCOUNT")
        );

        let small = probe_peak_rss("p5_rss_2000", "rss_2000", 2000);
        let large = probe_peak_rss("p5_rss_200000", "rss_200000", 200000);
        assert!(small > 0, "peak RSS of the small probe was not measured");
        let ratio = large as f64 / small as f64;
        assert!(
            ratio < 1.35,
            "peak RSS grew with the trip count: {small} -> {large} (ratio {ratio})"
        );
    }
}
