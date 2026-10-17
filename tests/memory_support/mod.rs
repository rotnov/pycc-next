//! The peak-RSS probe harness shared by the memory tests
//! (`tests/memory_leaks*.rs`, the `tests/issue_*` leak probes): build a probe
//! with `pycc build --release`, run it measuring its peak resident set, and
//! run CPython on the same source.
//!
//! # Why not `ru_maxrss`
//!
//! On Linux a child's `ru_maxrss` (from `wait4`, or its own
//! `getrusage(RUSAGE_SELF)`) is not the child's peak: `exec` folds the
//! high-water mark of the address space it replaces into the process's
//! `maxrss`, and that address space is the parent's (shared through
//! `vfork`, which `std::process::Command` uses, or copied by `fork`). So
//! `ru_maxrss` reads `max(the test harness's resident set at spawn, the
//! probe's own peak)`: a few-megabyte probe spawned by a harness running
//! tests in parallel reports the harness's size, and two probes compared
//! for growth compare how much the harness grew in between (a 1072 kB
//! "growth" against a 1024 kB limit, passing with `--test-threads=1`).
//!
//! The probe's own address space starts fresh at `exec`, and its high-water
//! mark is `VmHWM` in `/proc/<pid>/status`. It is read while the probe exits
//! but before its memory is torn down: the probe is traced
//! (`PTRACE_TRACEME`) with `PTRACE_O_TRACEEXIT`, which stops it at that
//! point. Elsewhere (macOS: `posix_spawn` gives the child a new task, and
//! `ru_maxrss` is in bytes there), or where tracing is not permitted, the
//! peak is `ru_maxrss`.
//!
//! On Windows a new process has an address space of its own from the
//! start, and its peak is `PeakWorkingSetSize` of `GetProcessMemoryInfo`,
//! read through the child's process handle after it exited (the handle
//! keeps the process object, and its counters, alive until it is closed).

#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// How much a probe's peak resident set may grow between its two loop
/// counts (20 000 and 220 000 iterations in most suites) without counting as
/// a leak.
///
/// Not tighter, because a probe embedding CPython grows in steps of up to
/// one obmalloc arena (1 MiB in CPython 3.13): whether the larger run
/// touches one more arena depends on fragmentation, not on a leak, and the
/// step shows up or not between neighbouring loop counts (the same json
/// probe measured ~16.1 MB at 50 000 iterations, ~16.9 MB at 100 000 and
/// 800 000, ~16.1 MB at 2 000 000). Not looser, because the smallest leak
/// worth catching -- one 16-byte object per iteration, more with its
/// allocator header -- adds at least 3.1 MB over 200 000 extra iterations.
pub const GROWTH_LIMIT_KB: i64 = 2048;

/// Python source defining `peak_kb()`, the running process's own peak
/// resident set in kilobytes, for a probe measuring from inside Python:
/// prepend it to the probe's source.
pub const PYTHON_PEAK_KB: &str = include_str!("../fixtures/peak_kb.py");

/// `ptrace`'s unused `addr`/`data` argument: pointer-sized, as the variadic
/// C prototype reads it.
#[cfg(target_os = "linux")]
const NULL: *mut libc::c_void = std::ptr::null_mut();

pub fn build(dir: &Path, name: &str, source: &str) -> PathBuf {
    build_with(dir, name, source, &[])
}

/// [`build`] with extra `pycc build` flags (`--non-strict`).
pub fn build_with(dir: &Path, name: &str, source: &str, flags: &[&str]) -> PathBuf {
    let src = dir.join(format!("{name}.py"));
    std::fs::write(&src, source).expect("write the probe source");
    let bin = dir.join(name);
    let output = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .arg("--release")
        .args(flags)
        .output()
        .expect("pycc should spawn");
    assert!(
        output.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    bin
}

/// A finished probe: its stdout, raw wait status and peak resident set.
pub struct Measured {
    pub stdout: Vec<u8>,
    /// The raw `waitpid` status (`libc::WIFEXITED` and friends apply); on
    /// Windows the process exit code.
    pub status: i32,
    /// The probe's own peak resident set, in kilobytes.
    pub peak_kb: i64,
}

impl Measured {
    #[cfg(unix)]
    pub fn success(&self) -> bool {
        libc::WIFEXITED(self.status) && libc::WEXITSTATUS(self.status) == 0
    }

    #[cfg(windows)]
    pub fn success(&self) -> bool {
        self.status == 0
    }
}

/// Runs `bin`, returning its stdout and peak resident set in kilobytes;
/// panics unless it exits with status 0.
pub fn run_measured(bin: &Path) -> (Vec<u8>, i64) {
    let measured = measure(Command::new(bin), None);
    assert!(
        measured.success(),
        "{} exited abnormally: {:#x}",
        bin.display(),
        measured.status
    );
    (lf(measured.stdout), measured.peak_kb)
}

/// The peak resident set, in kilobytes, of `command`, which must exit with
/// status 0 (its stdout is discarded).
pub fn peak_kb(command: Command) -> i64 {
    let shown = format!("{command:?}");
    let measured = measure(command, None);
    assert!(
        measured.success(),
        "command failed ({:#x}): {shown}",
        measured.status
    );
    measured.peak_kb
}

/// Runs `command` to completion -- with `input` on its stdin when given,
/// stdout captured, stderr as `command` configures it -- and measures its
/// own peak resident set (see the module documentation).
pub fn measure(mut command: Command, input: Option<&[u8]>) -> Measured {
    command.stdout(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `ptrace(PTRACE_TRACEME)` is async-signal-safe, as
        // `pre_exec` requires.
        unsafe {
            command.pre_exec(|| {
                if libc::ptrace(libc::PTRACE_TRACEME, 0, NULL, NULL) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        match command.spawn() {
            Ok(child) => return traced::finish(child, input),
            Err(error) if error.raw_os_error() == Some(libc::EPERM) => {
                eprintln!(
                    "memory_support: ptrace is not permitted ({error}); \
                     measuring ru_maxrss, which includes this process's resident set"
                );
                // A plain spawn below: the `pre_exec` hook stays on the
                // command, so rebuild it without one.
                command = untraced_copy(&command);
                command.stdout(Stdio::piped());
                if input.is_some() {
                    command.stdin(Stdio::piped());
                }
            }
            Err(error) => panic!("the probe should spawn: {error}"),
        }
    }
    finish(command.spawn().expect("the probe should spawn"), input)
}

/// Waits for the untraced `child` and reads its peak from `wait4`'s
/// `ru_maxrss`.
#[cfg(unix)]
fn finish(mut child: std::process::Child, input: Option<&[u8]>) -> Measured {
    let pid = child.id() as libc::pid_t;
    let stdout = collect_io(&mut child, input);
    let mut status = 0;
    // SAFETY: an all-zero `rusage` is a valid value for `wait4` to fill.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `pid` is our own unreaped child; both out-pointers are live.
    let waited = unsafe { libc::wait4(pid, &mut status, 0, &mut usage) };
    assert_eq!(waited, pid, "wait4 should reap the probe");
    let max_rss = usage.ru_maxrss;
    // `ru_maxrss` is in bytes on macOS and in kilobytes elsewhere.
    let peak_kb = if cfg!(target_os = "macos") {
        max_rss / 1024
    } else {
        max_rss
    };
    Measured {
        stdout: stdout.join().expect("the stdout reader"),
        status,
        peak_kb,
    }
}

/// A copy of `command` (program, arguments, environment, directory) without
/// its `pre_exec` hook.
#[cfg(target_os = "linux")]
fn untraced_copy(command: &Command) -> Command {
    let mut copy = Command::new(command.get_program());
    copy.args(command.get_args());
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => copy.env(key, value),
            None => copy.env_remove(key),
        };
    }
    if let Some(dir) = command.get_current_dir() {
        copy.current_dir(dir);
    }
    copy
}

/// Waits for `child` and reads its peak working set (the module docs).
#[cfg(windows)]
fn finish(mut child: std::process::Child, input: Option<&[u8]>) -> Measured {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    let stdout = collect_io(&mut child, input);
    let status = child.wait().expect("wait for the probe");
    // SAFETY: an all-zero `PROCESS_MEMORY_COUNTERS` is a valid value to fill.
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: the handle is `child`'s own, open until `child` drops; the
    // out-pointer is live and `size` is its size.
    let ok = unsafe { GetProcessMemoryInfo(child.as_raw_handle(), &mut counters, size) };
    assert_ne!(
        ok,
        0,
        "GetProcessMemoryInfo: {}",
        std::io::Error::last_os_error()
    );
    Measured {
        stdout: stdout.join().expect("the stdout reader"),
        status: status.code().unwrap_or(-1),
        peak_kb: (counters.PeakWorkingSetSize / 1024) as i64,
    }
}

/// Feeds `input` to `child`'s stdin and reads its stdout to the end, both on
/// threads of their own (a probe blocked on a full pipe must not stall the
/// thread that waits for it), returning the reader to join.
fn collect_io(
    child: &mut std::process::Child,
    input: Option<&[u8]>,
) -> std::thread::JoinHandle<Vec<u8>> {
    if let Some(input) = input {
        let mut stdin = child.stdin.take().expect("piped stdin");
        let input = input.to_vec();
        // Detached: it ends when the input is written or the probe exits.
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let mut stdout = child.stdout.take().expect("piped stdout");
    std::thread::spawn(move || {
        let mut out = Vec::new();
        stdout
            .read_to_end(&mut out)
            .expect("read the probe's stdout");
        out
    })
}

#[cfg(target_os = "linux")]
mod traced {
    use super::{Measured, NULL, collect_io};

    /// Drives the traced `child` to its exit, reading its `VmHWM` at the
    /// exit stop. Every `ptrace` request comes from this thread, the one
    /// that spawned the child: the tracer.
    pub(super) fn finish(mut child: std::process::Child, input: Option<&[u8]>) -> Measured {
        let pid = child.id() as libc::pid_t;
        let stdout = collect_io(&mut child, input);
        let mut status = wait(pid);
        // `PTRACE_TRACEME` stops the child with `SIGTRAP` at its `exec`.
        assert!(
            libc::WIFSTOPPED(status) && libc::WSTOPSIG(status) == libc::SIGTRAP,
            "the traced probe should stop at exec: {status:#x}"
        );
        let options = libc::PTRACE_O_TRACEEXIT | libc::PTRACE_O_TRACEEXEC | libc::PTRACE_O_EXITKILL;
        // SAFETY: `pid` is our tracee, stopped.
        let set = unsafe {
            libc::ptrace(
                libc::PTRACE_SETOPTIONS,
                pid,
                NULL,
                options as usize as *mut libc::c_void,
            )
        };
        assert_eq!(
            set,
            0,
            "PTRACE_SETOPTIONS: {}",
            std::io::Error::last_os_error()
        );
        resume(pid, 0);
        let mut peak_kb = None;
        loop {
            status = wait(pid);
            if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
                break;
            }
            assert!(
                libc::WIFSTOPPED(status),
                "unexpected wait status {status:#x}"
            );
            let event = (status >> 16) & 0xffff;
            let signal = libc::WSTOPSIG(status);
            if signal == libc::SIGTRAP && event == libc::PTRACE_EVENT_EXIT {
                peak_kb = Some(vm_hwm_kb(pid));
                resume(pid, 0);
            } else if signal == libc::SIGTRAP && event != 0 {
                // Another ptrace event (an `exec` of the probe's own).
                resume(pid, 0);
            } else {
                // A signal for the probe: deliver it.
                resume(pid, signal);
            }
        }
        Measured {
            stdout: stdout.join().expect("the stdout reader"),
            status,
            peak_kb: peak_kb.expect("the probe should stop at its exit"),
        }
    }

    fn wait(pid: libc::pid_t) -> libc::c_int {
        let mut status = 0;
        loop {
            // SAFETY: `pid` is our own child; `status` is live.
            let waited = unsafe { libc::waitpid(pid, &mut status, libc::__WALL) };
            if waited == pid {
                return status;
            }
            let error = std::io::Error::last_os_error();
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::Interrupted,
                "waitpid: {error}"
            );
        }
    }

    fn resume(pid: libc::pid_t, signal: libc::c_int) {
        // SAFETY: `pid` is our tracee, stopped.
        let resumed = unsafe {
            libc::ptrace(
                libc::PTRACE_CONT,
                pid,
                NULL,
                signal as usize as *mut libc::c_void,
            )
        };
        assert_eq!(
            resumed,
            0,
            "PTRACE_CONT: {}",
            std::io::Error::last_os_error()
        );
    }

    /// `VmHWM` of `/proc/<pid>/status`: the peak resident set of the
    /// address space `pid` got at its `exec`, in kilobytes.
    fn vm_hwm_kb(pid: libc::pid_t) -> i64 {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status"))
            .expect("read the probe's /proc status");
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmHWM:"))
            .and_then(|value| value.trim().strip_suffix("kB"))
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or_else(|| panic!("no VmHWM in /proc/{pid}/status:\n{status}"))
    }
}

pub fn cpython(src: &Path) -> Vec<u8> {
    let output = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg(src)
        .output()
        .expect("python3 should spawn");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    lf(output.stdout)
}

/// `bytes` with `\r\n` as `\n`: on Windows CPython (and a program writing
/// through its `sys.stdout`) ends lines with `\r\n`, a native program with
/// `\n` (docs/DEVIATIONS.md).
pub fn lf(bytes: Vec<u8>) -> Vec<u8> {
    if !bytes.contains(&b'\r') {
        return bytes;
    }
    String::from_utf8_lossy(&bytes).replace("\r\n", "\n").into_bytes()
}
