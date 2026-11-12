//! Pass 3's callgrind runs, each bounded by a deadline derived from what it
//! measures.
//!
//! A callgrind run has no natural end but the workload's: when valgrind is
//! much slower on a host than the 60 to 90 times its documentation and
//! section 2.3 assume, or hangs on the workload (as it did on GitHub's arm64
//! runner for pypng, past 45 minutes), one run would stall the whole profile.
//! So every run gets a deadline: CPython's, [`baseline_deadline`], from
//! CPython's median wall time for the workload (the probe pass 3 makes
//! anyway); each check's, [`check_deadline`], from how long CPython's own
//! callgrind run took. A run past its deadline is killed with every process
//! it started ([`output_within`]) and is an error, which `auto` turns into a
//! decision on wall time (`baseline`, `counted`).

use super::super::Driver;
use super::unpatchable;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How many times CPython's median wall time for the workload its callgrind
/// run may take. Callgrind runs a workload about 60 to 90 times slower than
/// CPython does natively (section 2.3, measured on x86_64); this allows over
/// three times that, for a slower host or valgrind port, before the run is
/// taken for one that will not finish.
pub(crate) const BASELINE_SLOWDOWN: u32 = 300;

/// How many times CPython's callgrind run a check's compiled copy may take
/// under callgrind. The compiled copy runs the same workload with some of its
/// functions compiled, so its count is within a few percent of CPython's in
/// every check measured, and a check that loses rarely loses more than
/// twice over; four times leaves room for that and for the host's noise.
pub(crate) const CHECK_SLOWDOWN: u32 = 4;

/// The shortest deadline any callgrind run gets: valgrind's own start and
/// the interpreter's start under it take seconds before the workload begins,
/// which a short workload's multiple would not cover.
pub(crate) const MIN_DEADLINE: Duration = Duration::from_secs(120);

/// How often [`output_within`] looks whether its process has exited.
const POLL: Duration = Duration::from_millis(50);

/// The deadline of CPython's callgrind run of a workload whose median wall
/// time in CPython is `wall_ns`.
pub(crate) fn baseline_deadline(wall_ns: u64) -> Duration {
    Duration::from_nanos(wall_ns.saturating_mul(u64::from(BASELINE_SLOWDOWN))).max(MIN_DEADLINE)
}

/// The deadline of a check's callgrind run, when CPython's took `baseline`.
pub(crate) fn check_deadline(baseline: Duration) -> Duration {
    baseline.saturating_mul(CHECK_SLOWDOWN).max(MIN_DEADLINE)
}

/// One run of the workload under callgrind, killed past `deadline`: its
/// instruction count and how long it took.
pub(super) fn instructions(
    driver: &Driver<'_>,
    out: &Path,
    workload: &[OsString],
    deadline: Duration,
) -> Result<(u64, Duration), String> {
    let file = format!("--callgrind-out-file={}", out.with_extension("cg").display());
    let wrapper: [&OsStr; 3] = ["valgrind".as_ref(), "--tool=callgrind".as_ref(), file.as_ref()];
    let mut command = driver.command("time", out, &[], workload, &wrapper);
    command.env("PYCC_REQUIRE_PATCH", "1");
    let started = Instant::now();
    let output = output_within(&mut command, deadline);
    let took = started.elapsed();
    let _ = std::fs::remove_file(out.with_extension("cg"));
    let output = output
        .map_err(|e| format!("counting instructions needs valgrind: {e}"))?
        .ok_or_else(|| format!("callgrind did not finish within {} s", deadline.as_secs()))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        unpatchable(&output.stderr)?;
        return Err(format!("the profile's time pass under callgrind failed ({}):\n{stderr}", output.status));
    }
    stderr
        .lines()
        .find_map(|line| line.split_once("Collected :").and_then(|(_, n)| n.trim().parse().ok()))
        .map(|n| (n, took))
        .ok_or_else(|| "callgrind reported no instruction count".to_string())
}

/// `command`'s output, as `Command::output` gives it, or `None` when it had
/// not exited within `deadline`: then it is killed with every process it
/// started. On Unix it runs in a process group of its own, which is killed
/// whole; on Linux it also dies with this process, as its own group no longer
/// receives the terminal's Ctrl-C.
pub(crate) fn output_within(command: &mut Command, deadline: Duration) -> std::io::Result<Option<Output>> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    own_group(command);
    let mut child = command.spawn()?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let stdout = stdout.join().unwrap_or_default();
            let stderr = stderr.join().unwrap_or_default();
            return Ok(Some(Output { status, stdout, stderr }));
        }
        let elapsed = started.elapsed();
        if elapsed >= deadline {
            // The group is killed before the child is reaped, so its id
            // still names it. The readers are left: a process that left the
            // group could hold the pipes open.
            kill_tree(&mut child);
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(POLL.min(deadline - elapsed));
    }
}

/// A thread reading `pipe` to its end.
fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

#[cfg(unix)]
fn own_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    #[cfg(target_os = "linux")]
    {
        unsafe extern "C" {
            fn prctl(option: std::ffi::c_int, ...) -> std::ffi::c_int;
            fn getppid() -> std::ffi::c_int;
        }
        const PR_SET_PDEATHSIG: std::ffi::c_int = 1;
        const SIGKILL: std::ffi::c_ulong = 9;
        let parent = std::process::id();
        // SAFETY: `prctl` and `getppid` are async-signal-safe, and the
        // closure allocates nothing.
        unsafe {
            command.pre_exec(move || {
                if prctl(PR_SET_PDEATHSIG, SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // The parent may have exited before the signal was asked for.
                if getppid() as u32 != parent {
                    return Err(std::io::Error::other("pycc exited"));
                }
                Ok(())
            });
        }
    }
}

#[cfg(not(unix))]
fn own_group(_: &mut Command) {}

#[cfg(unix)]
fn kill_tree(child: &mut std::process::Child) {
    unsafe extern "C" {
        fn kill(pid: std::ffi::c_int, sig: std::ffi::c_int) -> std::ffi::c_int;
    }
    const SIGKILL: std::ffi::c_int = 9;
    // SAFETY: the child leads its own process group, not yet reaped.
    if unsafe { kill(-(child.id() as std::ffi::c_int), SIGKILL) } != 0 {
        let _ = child.kill();
    }
}

#[cfg(not(unix))]
fn kill_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

#[cfg(test)]
#[path = "callgrind_tests.rs"]
mod tests;
