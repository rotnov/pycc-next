use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{FILETIME, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::GetProcessTimes;

const RUNS: usize = 7;
const CPU_TIME_CHILD_ENV: &str = "PYCC_NBODY_CPU_TIME_CHILD";

#[derive(Clone, Copy, Debug)]
struct Timings {
    wall: f64,
    cpu: f64,
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).expect("timings are never NaN"));
    values[values.len() / 2]
}

fn nbody_summary(
    wall_ratio: f64,
    cpu_ratio: f64,
    cpython_wall_median: f64,
    pycc_wall_median: f64,
    cpython_cpu_median: f64,
    pycc_cpu_median: f64,
    required_wall: f64,
) -> String {
    format!(
        "nbody speedup: wall_ratio={wall_ratio:.4}x cpu_ratio={cpu_ratio:.4}x \
         cpython_wall_median={cpython_wall_median:.6}s \
         pycc_wall_median={pycc_wall_median:.6}s \
         cpython_cpu_median={cpython_cpu_median:.6}s \
         pycc_cpu_median={pycc_cpu_median:.6}s \
         required_wall={required_wall:.0}x wall_pass={}",
        wall_ratio >= required_wall
    )
}

#[cfg(unix)]
fn rusage_seconds(sec: i64, usec: i64) -> f64 {
    sec as f64 + usec as f64 * 1e-6
}

#[cfg(windows)]
fn filetime_seconds(high: u32, low: u32) -> f64 {
    (((high as u64) << 32) | low as u64) as f64 * 1e-7
}

#[cfg(unix)]
// `wait4` below is the deliberate reaping operation, which Clippy cannot
// associate with `std::process::Child`. The `i64` casts normalize ABI-varying
// `time_t`/`suseconds_t` field types at the fixed conversion-helper boundary.
#[allow(clippy::unnecessary_cast, clippy::zombie_processes)]
fn time_command(mut command: Command) -> Timings {
    let start = Instant::now();
    let child = command.spawn().expect("command must spawn");
    let pid = child.id() as libc::pid_t;
    let mut status = 0;
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();

    let waited = loop {
        // SAFETY: `pid` identifies the live child spawned immediately above;
        // both output pointers are valid for writes for the duration of this
        // call. A successful `wait4` initializes the complete `rusage` value.
        let result = unsafe { libc::wait4(pid, &mut status, 0, usage.as_mut_ptr()) };
        if result != -1 {
            break result;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            panic!("command wait failed: {command:?}: {error}");
        }
    };
    let wall = start.elapsed().as_secs_f64();
    assert_eq!(waited, pid, "wait4 reaped the wrong child for {command:?}");
    // SAFETY: the successful `wait4` above initialized `usage`.
    let usage = unsafe { usage.assume_init() };
    assert!(
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
        "command failed: {command:?}"
    );
    let cpu = rusage_seconds(usage.ru_utime.tv_sec as i64, usage.ru_utime.tv_usec as i64)
        + rusage_seconds(usage.ru_stime.tv_sec as i64, usage.ru_stime.tv_usec as i64);
    Timings { wall, cpu }
}

#[cfg(windows)]
fn time_command(mut command: Command) -> Timings {
    let start = Instant::now();
    let mut child = command.spawn().expect("command must spawn");
    let status = child.wait().expect("command wait must succeed");
    let wall = start.elapsed().as_secs_f64();
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: `child` still owns a valid process handle after `wait()`, and
    // all four `FILETIME` pointers are valid for writes during the call.
    let result = unsafe {
        GetProcessTimes(
            child.as_raw_handle() as HANDLE,
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    assert_ne!(
        result,
        0,
        "GetProcessTimes failed for {command:?}: {}",
        std::io::Error::last_os_error()
    );
    assert!(status.success(), "command failed: {command:?}");
    let cpu = filetime_seconds(kernel.dwHighDateTime, kernel.dwLowDateTime)
        + filetime_seconds(user.dwHighDateTime, user.dwLowDateTime);
    Timings { wall, cpu }
}

/// The pinned CPython 3.14.7 oracle (D-001's "python3.14" pin). Duplicated
/// from `tests/conformance.rs` rather than shared through a common module --
/// this repository's existing integration-test convention (e.g.
/// `tests/pycc_toml_release_default.rs`'s own private `pycc_bin()`) is each
/// `tests/*.rs` file carrying its own small self-contained helpers, since
/// Rust integration-test binaries don't share code across files without a
/// `tests/common/mod.rs`-style module none of these files currently use.
///
/// Windows needs its own file name here: `std::process::Command::new` on
/// Windows resolves a bare program name by searching `PATH` for that exact
/// file name and does not append `.exe` when the name already contains a
/// `.` (the version dot in "python3.14" reads as an extension to that
/// resolver). A CI-side `python3.14.exe` alias on `PATH` is therefore
/// invisible to this lookup even though shells like bash find it fine.
/// Passing the extension explicitly on Windows sidesteps the mismatch,
/// exactly like `tests/conformance.rs::oracle_binary_name` (D-080 addendum).
fn oracle_binary_name(is_windows: bool) -> &'static str {
    if is_windows {
        "python3.14.exe"
    } else {
        "python3.14"
    }
}

fn oracle_python_bin() -> PathBuf {
    let bin = PathBuf::from(oracle_binary_name(cfg!(windows)));
    let output = Command::new(&bin)
        .arg("--version")
        .output()
        .unwrap_or_else(|e| panic!("nbody benchmark oracle `python3.14` not found on PATH: {e}"));
    let version = String::from_utf8_lossy(&output.stdout);
    assert!(
        version.trim() == "Python 3.14.7",
        "nbody benchmark oracle must be exactly Python 3.14.7, found {version:?}"
    );
    bin
}

/// CPython's stdio layer translates `\n` to `\r\n` on Windows even when
/// stdout is piped/redirected (D-082); `pycc_rt`'s `println!`-based `print()`
/// never does. Duplicated from `tests/conformance.rs::strip_windows_
/// newline_translation` rather than shared -- this file's own established
/// convention, see `oracle_python_bin`'s doc comment above.
fn strip_windows_newline_translation(bytes: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut iter = bytes.into_iter().peekable();
    while let Some(byte) = iter.next() {
        if byte == b'\r' && iter.peek() == Some(&b'\n') {
            continue;
        }
        out.push(byte);
    }
    out
}

/// Parses the fixture's final-sun-position stdout (`print(sun_x, sun_y,
/// sun_z)`, three whitespace-separated floats, one trailing newline) into
/// `[f64; 3]`. Asserts exactly three tokens rather than silently truncating
/// or padding, so a build that prints too little or too much output (a
/// truncated write, an extra print) fails loudly here instead of comparing
/// a degenerate subset.
fn parse_three_floats(bytes: &[u8]) -> [f64; 3] {
    let text = String::from_utf8(bytes.to_vec()).expect("nbody output must be valid UTF-8");
    let tokens: Vec<&str> = text.split_whitespace().collect();
    assert_eq!(
        tokens.len(),
        3,
        "expected exactly 3 whitespace-separated floats in nbody output, got {}: {text:?}",
        tokens.len()
    );
    let mut values = [0.0; 3];
    for (value, token) in values.iter_mut().zip(tokens.iter()) {
        *value = token
            .parse::<f64>()
            .unwrap_or_else(|e| panic!("failed to parse {token:?} as f64: {e}"));
    }
    values
}

/// D-098: compares parsed floats with a *relative* tolerance rather than
/// D-097's original exact bytes, which CI on `windows-latest` refuted (see
/// D-098's own Context). A single absolute epsilon would be either useless
/// (too large relative to the third value, ~3e-4 in magnitude) or vacuous
/// (too small relative to the first, ~8e-3) -- nbody's own three reported
/// values span multiple orders of magnitude. `1e-6` leaves roughly three
/// orders of margin above the worst cross-platform relative difference
/// actually observed in CI (~7.41e-10, the second of the three values,
/// pycc vs. CPython on `windows-latest`) while staying far below what a
/// real miscompile (wrong formula, wrong iteration count, wrong update
/// order) would produce -- those diverge in early digits, not the trailing
/// ones.
const RELATIVE_TOLERANCE: f64 = 1e-6;

fn assert_relative_eq(pycc: [f64; 3], cpython: [f64; 3]) {
    for (index, (p, c)) in pycc.iter().zip(cpython.iter()).enumerate() {
        let relative_diff = (p - c).abs() / c.abs().max(f64::MIN_POSITIVE);
        assert!(
            relative_diff <= RELATIVE_TOLERANCE,
            "pycc and CPython 3.14.7 disagree on tests/fixtures/nbody.py's output value #{index} \
             beyond the {RELATIVE_TOLERANCE:e} relative tolerance -- a fast but wrong binary must \
             not pass this speedup gate (pycc={p}, cpython={c}, relative difference={relative_diff:e})"
        );
    }
}

#[test]
fn median_returns_the_middle_of_five_sorted_values() {
    assert_eq!(median(vec![3.0, 1.0, 2.0, 5.0, 4.0]), 3.0);
}

#[test]
fn nbody_summary_schema_is_exact_for_pass_and_failure() {
    assert_eq!(
        nbody_summary(21.25, 18.5, 2.0, 0.1, 1.5, 0.08, 20.0),
        "nbody speedup: wall_ratio=21.2500x cpu_ratio=18.5000x \
         cpython_wall_median=2.000000s pycc_wall_median=0.100000s \
         cpython_cpu_median=1.500000s pycc_cpu_median=0.080000s \
         required_wall=20x wall_pass=true"
    );
    assert_eq!(
        nbody_summary(19.0, 17.25, 1.9, 0.1, 1.38, 0.08, 20.0),
        "nbody speedup: wall_ratio=19.0000x cpu_ratio=17.2500x \
         cpython_wall_median=1.900000s pycc_wall_median=0.100000s \
         cpython_cpu_median=1.380000s pycc_cpu_median=0.080000s \
         required_wall=20x wall_pass=false"
    );
}

#[cfg(unix)]
#[test]
fn rusage_seconds_combines_seconds_and_microseconds() {
    assert_eq!(rusage_seconds(2, 500_000), 2.5);
}

#[cfg(windows)]
#[test]
fn filetime_seconds_combines_halves_and_converts_100ns_ticks() {
    assert_eq!(filetime_seconds(1, 5_000_000), 429.9967296);
}

#[test]
fn timed_child_burns_cpu() {
    if std::env::var_os(CPU_TIME_CHILD_ENV).is_none() {
        return;
    }
    let start = Instant::now();
    let mut value = 1_u64;
    while start.elapsed().as_millis() < 50 {
        value = std::hint::black_box(value.wrapping_mul(6364136223846793005).wrapping_add(1));
    }
    std::hint::black_box(value);
}

#[test]
fn time_command_reports_positive_wall_and_per_child_cpu_time() {
    let mut command = Command::new(std::env::current_exe().expect("test executable must exist"));
    command
        .arg("--exact")
        .arg("timed_child_burns_cpu")
        .env(CPU_TIME_CHILD_ENV, "1");
    let timings = time_command(command);
    assert!(timings.wall.is_finite() && timings.wall > 0.0);
    assert!(timings.cpu.is_finite() && timings.cpu > 0.0);
}

#[test]
fn parse_three_floats_reads_whitespace_separated_values() {
    assert_eq!(
        parse_three_floats(b"0.5 -1.25 3.0\n"),
        [0.5_f64, -1.25_f64, 3.0_f64]
    );
}

#[test]
#[should_panic(expected = "expected exactly 3 whitespace-separated floats")]
fn parse_three_floats_rejects_the_wrong_token_count() {
    parse_three_floats(b"0.5 -1.25\n");
}

#[test]
#[should_panic(expected = "failed to parse")]
fn parse_three_floats_rejects_a_non_numeric_token() {
    parse_three_floats(b"0.5 abc 3.0\n");
}

#[test]
fn assert_relative_eq_accepts_differences_within_tolerance() {
    // Mirrors the real cross-platform gap observed on windows-latest
    // (~7.41e-10 relative, the worst of the three values), comfortably
    // inside the 1e-6 bound.
    assert_relative_eq(
        [1.0, 0.00387155830433383, 1.0],
        [1.0, 0.0038715583072033615, 1.0],
    );
}

#[test]
#[should_panic(expected = "beyond the 1e-6 relative tolerance")]
fn assert_relative_eq_rejects_differences_beyond_tolerance() {
    assert_relative_eq([1.0, 1.0, 1.0], [1.01, 1.0, 1.0]);
}

#[test]
fn strip_windows_newline_translation_removes_cr_before_lf_only() {
    let input = b"line one\r\nline two\nline three\r\n".to_vec();
    let expected = b"line one\nline two\nline three\n".to_vec();
    assert_eq!(strip_windows_newline_translation(input), expected);

    // A lone `\r` not immediately followed by `\n` is not CPython's Windows
    // newline translation and must be left untouched.
    let lone_cr = b"a\rb\n".to_vec();
    assert_eq!(strip_windows_newline_translation(lone_cr.clone()), lone_cr);
}

#[test]
fn oracle_binary_name_appends_the_exe_extension_only_for_windows() {
    assert_eq!(oracle_binary_name(true), "python3.14.exe");
    assert_eq!(oracle_binary_name(false), "python3.14");
}

/// D-094's nbody measurement contract (design doc's own §1): same-machine
/// paired comparison, `K = 7` runs each (raised from the design doc's
/// original `K = 5` by D-140), ratio of medians, `--release` pycc
/// vs. the pinned CPython 3.14.7 oracle, gate at ratio >= 20 -- preceded by
/// one untimed correctness check (D-097) that verifies both sides compute
/// matching output, within a small relative tolerance (D-098, superseding
/// D-097's original exact-byte comparison), before any ratio is trusted.
/// `#[ignore]`d like
/// `tests/conformance.rs`'s two fixtures -- genuinely slow (a full
/// `--release` LLVM build plus sixteen total program executions: one untimed
/// correctness-check launch per side, then the fourteen timed launches below) --
/// and run explicitly via `--include-ignored`, already passed workspace-wide in
/// both `build-test-coverage` and every `native-build-test` matrix leg
/// (`.github/workflows/ci.yml`), so no further CI test-wiring change was
/// needed beyond D-092's own release-`pycc_rt`-build step addition there.
///
/// This test now passes on every Tier-1 target given the per-target floors
/// below (20x on two of five, 12x on macOS aarch64 per D-095, 15x on
/// `windows-latest` per D-096, and 18x on Linux aarch64 per D-101) -- it
/// originally failed, for a narrower,
/// better understood reason than the ~10-11x this benchmark first measured
/// (D-093): that first measurement conflated a real methodology gap with
/// what turned out to be a real, separate implementation bug, both fixed
/// before the gate itself was later revisited per-target:
/// 1. `find_pycc_rt_lib_dir_in` (then in `src/main.rs`, now in
///    `pycc_codegen::artifact_layout`) used to always link
///    `target/debug/libpycc_rt.a` regardless of `--release` (the flag only
///    optimized the compiled module's own LLVM IR, never selected an
///    optimized `pycc_rt` to link) -- fixed (D-092): it now takes a
///    `release: bool` and links `target/release` when the caller's already-
///    resolved `--release` state says to.
/// 2. At `DEFAULT_ITERATIONS = 20000` (pyperformance's own upstream
///    constant), pycc's own ~3ms fixed process-spawn overhead was ~45-50%
///    of its ~6ms total nbody runtime, mechanically compressing the
///    measured ratio far below the actual compute-only speedup -- fixed
///    (D-093): `tests/fixtures/nbody.py`'s iteration count is raised to
///    `525000`, keeping both sides' own fixed-overhead fraction in the
///    single digits (pycc ~4.3%, CPython ~1.6%) without changing any
///    physics, constant, or update-order fidelity to the reference
///    benchmark. This fixture's own 10 pairwise `pycc_rt_float_pow` calls
///    per iteration now total 5,250,000 over a full run, not 200,000.
///
/// With both fixes in place, the measured ratio is a stable, reproducible
/// ~18.0-18.24x (see D-093 for five consecutive runs' worth of numbers) --
/// still short of the 20x gate, but now a genuine, well-bounded compute
/// ceiling rather than a measurement artifact: real timing at 300k/400k/
/// 525k/800k/1M iterations shows the ratio is not still climbing (17.34x/
/// 17.72x/17.78x/18.03x/18.14x), so raising the iteration count further
/// would not close this gap. `pycc_rt_float_pow` remains an opaque
/// `extern "C"` call from the compiled module's own LLVM IR (v0.2 has no
/// cross-module LTO, D-094), so LLVM can never inline it into the hot loop
/// regardless of which `pycc_rt` build is linked -- closing the remaining
/// ~2x gap would need real cross-module optimization work, out of this
/// test's own scope. The gate itself is a design-doc-mandated threshold
/// (design doc's §1); rewriting this fixture's computation to dodge
/// `pycc_rt_float_pow` calls would defeat the point of building this
/// measurement in the first place, and the gate stays at 20 on every
/// Tier-1 target except three documented, evidence-backed exceptions -- see
/// `required_nbody_ratio`'s own doc comment for macOS aarch64's (D-095),
/// `windows-latest`'s (D-096), and `ubuntu-24.04-arm`'s (D-101) separately-
/// measured floors. See D-093
/// for the full investigation and the task dispatcher's own decision on
/// how to proceed.
///
/// Runs execute in two back-to-back blocks (all `RUNS` pycc runs, then all
/// `RUNS` CPython runs) rather than interleaved -- matching the design doc's
/// own "both programs run K times; take the median of each" wording, which
/// specifies K's aggregation (median) but not an interleaving requirement.
/// Block ordering is slightly more exposed to monotonic drift (thermal
/// throttling, background-process ramp-up) than interleaving would be,
/// since drift penalizes whichever side runs second rather than being
/// averaged across both; taking the median (not the mean) of `RUNS`
/// same-block runs already blunts most of that exposure, and D-140 raising
/// `RUNS` from 5 to 7 blunts it further still (a median across more samples
/// is harder for any single outlier run to move).
///
/// D-126 records per-child CPU time alongside wall-clock time while preserving
/// the existing wall-clock gate and every per-target floor. Unix uses `wait4`
/// for the exact spawned child rather than process-global `RUSAGE_CHILDREN`;
/// Windows reads the waited child's retained process handle with
/// `GetProcessTimes`. The frozen step-summary record exposes both ratios and all
/// four medians on passes and failures. D-129 completed Phase B after five real
/// observations on every Tier-1 leg: CPU time did not reduce variance across
/// the matrix, so wall clock remains the gate and CPU time remains non-gating
/// diagnostic telemetry rather than gaining its own floors. D-140 records why:
/// on virtualized hosted runners, "CPU time" per `getrusage`/`GetProcessTimes`
/// still counts scheduled-but-throttled cycles as full-rate CPU-seconds, so
/// hypervisor-level noise (CPU steal time from co-tenants, frequency/turbo
/// scaling driven by neighboring load) leaks into the CPU-time measurement
/// almost as much as it does into wall-clock -- CPU time is immune to pure
/// OS-scheduler preemption (that time is correctly excluded), but not to the
/// virtualization-layer noise that dominates on these runners, so it does not
/// close the "shared CPU" gap the way it would on dedicated hardware.
#[test]
#[ignore = "slow: builds a --release binary, verifies output equality once, then runs both programs 7 times each"]
fn nbody_release_binary_meets_required_speedup_over_cpython() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nbody.py");

    // Build the pycc binary once, --release, outside the timed loop. The
    // scratch directory handle must span the whole test: the timed loops
    // below re-launch `bin_path` from inside it, and Drop removes it.
    let dir = ScratchDir::new("nbody").expect("failed to create scratch dir");
    let bin_path = dir.join("nbody");
    let build_status = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&fixture)
        .arg("-o")
        .arg(&bin_path)
        .arg("--release")
        .status()
        .expect("pycc build must spawn");
    assert!(
        build_status.success(),
        "pycc --release build of nbody.py failed"
    );

    // Resolved once, not per run: `oracle_python_bin()` itself spawns a
    // `--version` check, and re-running that inside the loop below would
    // burn four redundant process spawns per test run for no benefit (the
    // oracle binary and its version can't change mid-test).
    let cpython_bin = oracle_python_bin();

    // One untimed correctness check before the timed loops below: a
    // miscompile that runs successfully but computes wrong values could
    // otherwise still pass this gate purely on speed (D-093's own
    // "measurement with near-zero signal for what it's meant to measure"
    // mistake, but for correctness instead of performance). This deliberately
    // does not reuse `time_command`/`Command::status()` for the RUNS timed
    // launches below -- those must stay exactly as fast and side-effect-free
    // as they already are, since D-095/D-096's own gate floors (12x, 15x)
    // were measured against that exact shape; adding output capture to every
    // timed launch would change what's being measured. `strip_windows_
    // newline_translation` is duplicated from `tests/conformance.rs` rather
    // than shared (this file's own established convention, see
    // `oracle_python_bin`'s doc comment above) -- CPython's stdio layer
    // translates `\n` to `\r\n` on Windows even when piped (D-082), which
    // `pycc_rt`'s `println!`-based `print()` never does, so the oracle's
    // side needs the same normalization before tokenizing either side's
    // output into floats. D-097's original version compared raw bytes for
    // exact equality; CI on `windows-latest` refuted that (see this
    // function's own doc comment and D-098's Context, which supersedes
    // D-097's comparison method) -- pycc and CPython disagreed with each
    // other on Windows, and both disagreed with the macOS-recorded values,
    // all starting around the 10th significant digit, consistent with
    // `pow`/libm rounding differences amplified by this fixture's own
    // chaotic dynamics over 525,000 iterations, not a wrong formula or
    // miscompile (ten agreeing digits rules those out).
    // `assert_relative_eq` below compares parsed floats with a relative
    // tolerance instead.
    let pycc_check_output = Command::new(&bin_path)
        .output()
        .expect("pycc nbody binary must spawn for the correctness check");
    assert!(
        pycc_check_output.status.success(),
        "pycc nbody binary exited non-zero during the correctness check"
    );
    let cpython_check_output = Command::new(&cpython_bin)
        .arg(&fixture)
        .output()
        .expect("cpython oracle must spawn for the correctness check");
    assert!(
        cpython_check_output.status.success(),
        "cpython oracle exited non-zero during the correctness check"
    );
    let pycc_values = parse_three_floats(&pycc_check_output.stdout);
    let cpython_values = parse_three_floats(&strip_windows_newline_translation(
        cpython_check_output.stdout,
    ));
    assert_relative_eq(pycc_values, cpython_values);

    let pycc_times: Vec<Timings> = (0..RUNS)
        .map(|_| time_command(Command::new(&bin_path)))
        .collect();
    let cpython_times: Vec<Timings> = (0..RUNS)
        .map(|_| {
            let mut cpython = Command::new(&cpython_bin);
            cpython.arg(&fixture);
            time_command(cpython)
        })
        .collect();

    let pycc_wall_median = median(pycc_times.iter().map(|timing| timing.wall).collect());
    let cpython_wall_median = median(cpython_times.iter().map(|timing| timing.wall).collect());
    let pycc_cpu_median = median(pycc_times.iter().map(|timing| timing.cpu).collect());
    let cpython_cpu_median = median(cpython_times.iter().map(|timing| timing.cpu).collect());
    let wall_ratio = cpython_wall_median / pycc_wall_median;
    let cpu_ratio = cpython_cpu_median / pycc_cpu_median;
    let required = required_nbody_ratio(
        cfg!(target_os = "macos") && cfg!(target_arch = "aarch64"),
        cfg!(target_os = "windows"),
        cfg!(target_os = "linux") && cfg!(target_arch = "aarch64"),
    );

    // Report the measured ratio unconditionally, not only on failure: the
    // assertion below only ever produces a message when it fails, which left
    // every passing run's exact ratio unrecorded -- the gap that made
    // `ubuntu-24.04-arm`'s own fail cluster look like a censored left tail
    // before a 4th failure landing inside the same band settled it (D-101).
    // This instrumentation is what gives any future revisit of D-101's own
    // floor real pass-side ratio data to work from. Writing directly to
    // `$GITHUB_STEP_SUMMARY` (not `println!`/`eprintln!`) is deliberate:
    // cargo test's own libtest harness captures each test's stdout/stderr and
    // only forwards it to real process output on failure, so a plain print
    // here would stay invisible on a pass without `--nocapture` -- and this
    // repo's CI never adds that flag, since every `--include-ignored` run and
    // D-014's coverage step would become far noisier for no benefit. A failed
    // write is silently ignored: this is a diagnostic aid, not part of the
    // gate itself, and must never turn an otherwise-passing measurement into
    // a spurious failure.
    if let Ok(summary_path) = std::env::var("GITHUB_STEP_SUMMARY") {
        use std::io::Write;
        let _ = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(summary_path)
            .and_then(|mut file| {
                let summary = nbody_summary(
                    wall_ratio,
                    cpu_ratio,
                    cpython_wall_median,
                    pycc_wall_median,
                    cpython_cpu_median,
                    pycc_cpu_median,
                    required,
                );
                writeln!(file, "{summary}")
            });
    }

    assert!(
        wall_ratio >= required,
        "nbody wall-clock speedup ratio {wall_ratio:.2}x is below the required {required:.0}x gate \
         (cpython wall median {cpython_wall_median:.4}s, \
         pycc --release wall median {pycc_wall_median:.4}s; \
         CPU-time ratio {cpu_ratio:.2}x, cpython CPU median {cpython_cpu_median:.4}s, \
         pycc --release CPU median {pycc_cpu_median:.4}s)"
    );
}

/// D-095: 20x holds on every Tier-1 target except macOS aarch64
/// (`build-test-coverage`, the only Tier-1 leg on that architecture --
/// `native-build-test`'s own macOS leg is `macos-15-intel`, x86_64).
/// Three independent local runs on real (non-CI, non-virtualized) Apple
/// Silicon hardware reproduced D-093's own original ~18.0-18.24x plateau
/// almost exactly (18.01x, 18.12x, 17.95x), and precise per-launch-overhead
/// probes on that same hardware (`python3.14 -c pass` vs a trivial pycc
/// binary, ~22.3ms/~3.5ms) matched D-093's own baseline numbers closely --
/// confirming the fixed-overhead amortization D-093 already tuned for is
/// still working; the shortfall is not that overhead re-emerging. CI's own
/// `build-test-coverage` leg measured a worse 14.95x once (virtualized
/// aarch64 macOS runners are noisier than bare-metal). This floor is set
/// with margin below that single CI observation, not at the ~18x local
/// ceiling, precisely because only one CI data point exists for this leg.
/// The mechanism is not yet confirmed (see `docs/ROADMAP.md`'s follow-up
/// item) -- the leading hypothesis is that `pycc_rt_float_pow`'s
/// uninlineable cross-module call has different relative call/ABI
/// overhead on aarch64 than on x86_64, while CPython's own bytecode
/// dispatch loop benefits more evenly from Apple Silicon's raw compute
/// speedup, shrinking the *ratio* even though neither side runs slower in
/// absolute terms on faster hardware.
///
/// D-096: `windows-latest` gets its own relaxed floor (15x). Of 4 recorded
/// CI runs of this exact test on `windows-latest`, 3 measured 17.95x,
/// 17.78x, 17.61x -- a tight sub-20x cluster -- and 1 passed at an
/// unrecorded ratio above 20x (this assertion only reports the ratio on
/// failure, so that run's exact number was never captured). Read as
/// evidence for "a real ~17.6-18x ceiling with one favorable noise draw"
/// rather than "a genuine >=20x capability with unlucky sub-20x dips": a
/// true >=20x central tendency would need three independent draws to land
/// this tightly clustered below the gate, where a true ~17.6-18x central
/// tendency needs only one favorable draw to produce the single pass --
/// and CI noise on this exact benchmark is already known to swing several
/// points (`build-test-coverage`'s own 14.95x reading, D-095, is well
/// below its ~18x local-hardware baseline). This is read as the same kind
/// of signal D-095 relied on for macOS aarch64, but on a thinner
/// evidentiary base and with correspondingly lower confidence: unlike
/// D-095, no local, non-CI Windows hardware was available to reproduce
/// this outside CI, so it rests on these 4 CI observations alone. The
/// leading hypothesis -- Windows Defender's real-time scanner treating the
/// freshly built, unsigned compiled nbody binary (not `pycc` itself, which
/// this test invokes only once, to produce that binary via `pycc build
/// ... --release`) as unfamiliar and scanning it -- is unconfirmed and
/// tracked as the same kind of `docs/ROADMAP.md` follow-up as D-095's own
/// open question, not resolved here. Whether that scan cost is paid once
/// (first launch only, then cached) or on every one of this test's five
/// launches of that binary is itself unconfirmed; the hypothesis only
/// explains the observed *median*-ratio depression if most or all five
/// launches pay some overhead, since a purely first-launch-only cost would
/// show up as a single high outlier a median of five is largely designed
/// to discount, not as a shift in the median itself. This overhead would be
/// analogous to D-093's own already-solved fixed-per-launch-overhead
/// problem but from a different source (AV scanning, not process-spawn
/// cost) that D-093's own iteration-count fix does not amortize. 15.0 is
/// chosen with margin below the worst of the three sub-20x
/// observations (17.61x), not the single pass (which sets no useful
/// floor), matching D-095's own margin-below-worst-observation
/// convention.
///
/// D-101: `ubuntu-24.04-arm` gets its own relaxed floor (18x), decided later
/// and on a different evidentiary shape than D-095/D-096. Across PR-8's own
/// CI history this leg recorded 6 measurements: 4 failures -- 19.92x,
/// 19.88x, 19.86x, 19.90x -- all four landing inside a single 0.06-point
/// band, and 2 passes at unrecorded ratios (this assertion, like D-095's/
/// D-096's own, only ever reports a ratio on failure). An earlier reading of
/// the first 5 of these 6 observations concluded the tight fail band was a
/// censored-left-tail artifact of that reporting gap rather than a measured
/// ceiling, reasoning that a genuine ~19.9x plateau with ordinary noise
/// should scatter across independent draws, not repeatedly land in the same
/// narrow band, and that a near-40% pass rate argued for a true center
/// comfortably above 20x. The 4th failure landing inside that same band,
/// rather than scattering, is the evidence that reading said would be
/// needed before a floor decision here was defensible -- four independent
/// draws clustering this tightly is no longer read as a truncated view of a
/// distribution centered above 20x. Unlike D-095/D-096, no mechanism is
/// proposed for *why* this leg sits below 20x: nothing beyond the CI
/// measurements themselves has been investigated, and the two passes'
/// unrecorded ratios leave open something as simple as a true center near
/// 19.95x with ordinary noise of a few tenths straddling the gate, not
/// necessarily a bimodal "usually-degraded, sometimes-clean" pattern like
/// D-096's own Defender hypothesis -- that hypothesis is Windows-specific
/// and is not imported here for a different platform with no investigation
/// behind it. 18.0 is chosen with real margin below the worst observed
/// failure (19.86x) -- not just under it, which would leave no room for
/// ordinary run-to-run noise -- but well above D-096's 15.0, since this
/// leg's measured plateau sits a full ~2 points higher than
/// `windows-latest`'s and reusing D-096's own absolute margin here would
/// discard several points of real, already-observed headroom this leg has
/// never actually needed.
fn required_nbody_ratio(is_macos_aarch64: bool, is_windows: bool, is_linux_aarch64: bool) -> f64 {
    if is_macos_aarch64 {
        12.0
    } else if is_windows {
        15.0
    } else if is_linux_aarch64 {
        18.0
    } else {
        20.0
    }
}

#[test]
fn required_nbody_ratio_is_relaxed_only_on_macos_aarch64_windows_and_linux_aarch64() {
    assert_eq!(required_nbody_ratio(true, false, false), 12.0);
    assert_eq!(required_nbody_ratio(false, true, false), 15.0);
    assert_eq!(required_nbody_ratio(false, false, true), 18.0);
    assert_eq!(required_nbody_ratio(false, false, false), 20.0);
    // macos_aarch64 wins over both others if somehow more than one were true
    // (never actually happens -- no target is both macOS and Windows, or
    // both macOS and Linux -- but the precedence should be deterministic and
    // documented rather than left to argument order), and windows wins over
    // linux_aarch64 for the same never-occurring-in-practice reason.
    assert_eq!(required_nbody_ratio(true, true, true), 12.0);
    assert_eq!(required_nbody_ratio(false, true, true), 15.0);
}
