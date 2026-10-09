//! Shared RAII scratch-directory abstraction for pycc's production and test
//! code (issue #781, Part 1 of #779's 5-part decomposition).
//!
//! #779 found that ad hoc `std::env::temp_dir().join(...)` call sites across
//! ~36 test files (plus two production sites in `src/main.rs`, fixed
//! separately in Part 3) leaked directories whenever a test panicked before
//! its own manual `remove_dir_all` cleanup ran, and could collide when two
//! call sites in the same process picked the same name. [`ScratchDir`] fixes
//! both: `Drop`-based cleanup that runs even during a panic unwind, and a
//! naming scheme that cannot collide within a process and is vanishingly
//! unlikely to collide across processes.
//!
//! This crate intentionally has no dependencies beyond `std`. See the
//! decision record for why `tempfile`/`rand` were not added instead.
//!
//! Part 4 of #779 (issue #784) adds the second half of the lifecycle story:
//! every root carries a [`LOCK_FILE_NAME`] liveness marker held under an OS
//! advisory lock for the handle's lifetime, and [`sweep_stale_roots`]'s
//! bounded, best-effort sweep removes roots whose creating process is
//! provably dead (see the sweep module's docs for the exact safety bar).

mod sweep;

pub use sweep::{SweepReport, sweep_stale_roots};

use std::fs::File;
use std::io;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Name of the liveness-marker file [`ScratchDir::new`] creates inside every
/// fresh scratch root, shared with the probe behind [`sweep_stale_roots`]
/// and with tests that build sweep fixtures by hand.
///
/// The creating process holds an exclusive OS advisory lock
/// ([`File::lock`]) on this file for the lifetime of the [`ScratchDir`]
/// handle. The kernel releases the lock when the owning process exits by
/// *any* means — including `SIGKILL`, where no `Drop` runs — so a
/// successful [`File::try_lock`] on this file from another process is exact
/// proof that the creator is dead, immune to PID reuse by construction
/// (the lock belongs to the open file description, not to the PID).
pub const LOCK_FILE_NAME: &str = ".pycc-scratch.lock";

/// Per-process counter that disambiguates two [`ScratchDir::new`] calls that
/// land on the same nanosecond tick (possible on platforms where
/// `SystemTime`'s resolution is coarser than a nanosecond, and routine when
/// several threads race to create a scratch directory at once — exactly the
/// scenario the parallel-creation regression test below exercises).
static SEQ: AtomicU64 = AtomicU64::new(0);

/// RAII handle for a uniquely-named, pycc-owned scratch directory created
/// under the OS temp directory (`std::env::temp_dir()`).
///
/// Derefs to [`Path`], so it can be used anywhere a `&Path` is expected
/// (`scratch_dir.join("out.o")`, passed by reference, etc.) without first
/// unwrapping to the inner `PathBuf`.
///
/// `Drop` removes the directory tree, including while a panic is unwinding
/// through the stack frame that owns the handle — `std::fs::remove_dir_all`
/// is called unconditionally from `drop`, which Rust runs on every unwind
/// path, not only on ordinary scope exit.
///
/// # Naming and stability
///
/// Every directory this type creates is named
/// `pycc_{category}_{pid}_{nanos}_{seq}` under `std::env::temp_dir()`, where:
///
/// - `pid` is `std::process::id()`,
/// - `nanos` is the **full epoch nanosecond count** (a `u128` from
///   `SystemTime::now().duration_since(UNIX_EPOCH)`'s `as_nanos()`), not just
///   the sub-second remainder — the field must span real time so that two
///   process restarts a few seconds apart that happen to reuse the same PID
///   still get distinguishable names. A wall clock set before the Unix
///   epoch degrades this field to `0` rather than panicking — `build`/`run`
///   promise an actionable exit-2 environment error, never a panic, and a
///   real collision under a degraded clock still surfaces as
///   `create_dir`'s `AlreadyExists` error through the fallible path below,
/// - `seq` is `SEQ`, a private per-process atomic counter that rules out any
///   same-process collision even when two calls land on the same nanosecond.
///
/// This field order and format are a stability commitment with a real
/// consumer since Part 4 (#784): [`sweep_stale_roots`] parses a
/// directory name back into its fields without a live handle and treats a
/// full-format match as proof of pycc ownership. Changing the format is a breaking
/// change for that consumer. Note the parsed numbers are used for
/// *validation only* — a pre-epoch clock degrades `nanos` to `0`, so the
/// sweep never treats it as a plausible creation time; staleness comes from
/// filesystem mtime and liveness from the lock file below.
///
/// # Liveness marker
///
/// Every fresh root also contains a [`LOCK_FILE_NAME`] file on which the
/// handle holds an exclusive advisory lock (see that constant's docs).
/// Lock *acquisition* failure is deliberately ignored: on a filesystem
/// without advisory-lock support the root simply degrades to the sweep's
/// age-floor-only protection instead of failing the user's build — failing
/// a build to protect a janitor would invert priorities.
pub struct ScratchDir {
    path: PathBuf,
    /// Keeps the lock-file handle — and with it the advisory lock — alive
    /// exactly as long as the directory itself. `Option` so `Drop` can
    /// close the handle *before* `remove_dir_all`: on Windows an open
    /// handle inside the tree can block removal of its parent.
    lock: Option<File>,
}

impl ScratchDir {
    /// Creates a fresh, guaranteed-unique directory named
    /// `pycc_{category}_{pid}_{nanos}_{seq}` under `std::env::temp_dir()`
    /// and returns a handle that removes it on `Drop`.
    ///
    /// `category` is a short, caller-supplied label describing what the
    /// scratch directory is for (e.g. `"codegen_test"`, `"obj"`, `"run"`).
    /// It must be a short, static-ish string literal chosen by code inside
    /// this repository — never derived from external, user-, or
    /// file-supplied input — since it is spliced directly into the
    /// directory name without validation against the fixed-field format
    /// above. This is a caller discipline, not an enforced invariant: no
    /// workspace crate has a caller that needs to pass anything else, so a
    /// validation branch here would be dead code under the project's 100%
    /// coverage gate.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`io::Error`] if `std::fs::create_dir` fails —
    /// for example, a full disk, a permissions failure, or (deliberately
    /// unsupported input) a `category` containing a byte that is invalid in
    /// a path component on the current platform, such as a NUL byte on
    /// every platform this project targets. There is no retry: `create_dir`
    /// already fails atomically if the path already exists, and the
    /// `pid`/`nanos`/`seq` combination makes a same-process collision
    /// impossible and a cross-process collision unreachable in practice, so
    /// there is no genuine collision case for a retry loop to handle.
    /// Failing to create the [`LOCK_FILE_NAME`] liveness marker inside the
    /// fresh directory is also an error: the just-created directory is
    /// removed again (best-effort) and the creation error propagates.
    pub fn new(category: &str) -> io::Result<Self> {
        Self::new_with_lock_file_name(category, LOCK_FILE_NAME)
    }

    /// The one shared creation path. `lock_file_name` is injectable only so
    /// a test can force the lock-file creation to fail portably (a name
    /// under a nonexistent subdirectory fails with `NotFound` on every
    /// target) — production code always passes [`LOCK_FILE_NAME`] via
    /// [`ScratchDir::new`].
    fn new_with_lock_file_name(category: &str, lock_file_name: &str) -> io::Result<Self> {
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);

        let path = std::env::temp_dir().join(format!("pycc_{category}_{pid}_{nanos}_{seq}"));
        std::fs::create_dir(&path)?;
        let lock = match File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path.join(lock_file_name))
        {
            Ok(file) => file,
            Err(e) => {
                // Don't leave a fresh, markerless root behind on the error
                // path; it holds nothing yet, so plain `remove_dir`
                // suffices and its own failure is deliberately ignored
                // (the propagated creation error is the actionable one).
                let _ = std::fs::remove_dir(&path);
                return Err(e);
            }
        };
        // Acquisition failure deliberately ignored — see the type-level
        // "Liveness marker" docs. A single always-executed statement, so no
        // untestable error arm exists under the coverage gate.
        let _ = lock.lock();
        Ok(ScratchDir {
            path,
            lock: Some(lock),
        })
    }
}

impl Deref for ScratchDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        // Close the lock handle *before* removing the tree: on Windows an
        // open handle to a file inside the directory can make removal of
        // the parent fail; on unix the ordering is free. Closing also
        // releases the advisory lock, which is harmless mid-`Drop` — the
        // sweep's 1 h age floor covers the window, and a sweeper racing
        // this removal is deleting the same root anyway.
        drop(self.lock.take());
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic;
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::thread;

    #[test]
    fn the_directory_exists_while_the_handle_is_alive_and_is_gone_after_a_normal_drop() {
        let dir = ScratchDir::new("lib_normal_drop").expect("ScratchDir::new should succeed");
        let path = dir.to_path_buf();
        assert!(path.is_dir(), "ScratchDir::new must create the directory");
        drop(dir);
        assert!(
            !path.exists(),
            "Drop must remove the scratch directory on normal scope exit"
        );
    }

    #[test]
    fn the_directory_is_removed_even_when_a_panic_unwinds_through_the_handle() {
        let dir = ScratchDir::new("lib_panic_unwind").expect("ScratchDir::new should succeed");
        let path = dir.to_path_buf();
        assert!(path.is_dir(), "ScratchDir::new must create the directory");
        // `dir` is moved into the closure by value (not merely referenced),
        // so its `Drop` genuinely runs while the panic is unwinding through
        // this stack frame -- proving cleanup happens on the unwind path
        // itself, not just via ordinary end-of-scope drop after
        // `catch_unwind` returns.
        let result = panic::catch_unwind(panic::AssertUnwindSafe(move || {
            let _dir = dir;
            panic!("simulated test failure while the scratch directory is still live");
        }));
        assert!(result.is_err(), "the inner closure must have panicked");
        assert!(
            !path.exists(),
            "Drop must remove the scratch directory even when unwinding through a panic"
        );
    }

    #[test]
    fn concurrent_creation_with_the_same_category_from_many_threads_never_collides() {
        // 32 threads race to call `ScratchDir::new("lib_parallel")` at
        // (as close as possible to) the same instant, using a `Barrier` to
        // force the race. The `seq` counter -- not exact nanosecond timing
        // -- is what actually guarantees uniqueness under concurrency, so
        // this assertion holds regardless of how tightly the threads
        // actually synchronize. This is the scenario that the previous
        // PID-only naming scheme (`pycc_codegen_test_{label}_{pid}`, no
        // counter, no timestamp) got wrong: two calls with the same label
        // in the same process shared one PID and therefore one name.
        const THREADS: usize = 32;
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    ScratchDir::new("lib_parallel").expect("ScratchDir::new should not collide")
                })
            })
            .collect();

        let dirs: Vec<ScratchDir> = handles
            .into_iter()
            .map(|h| h.join().expect("worker thread should not panic"))
            .collect();

        let mut paths: Vec<PathBuf> = dirs.iter().map(|d| d.to_path_buf()).collect();
        for path in &paths {
            assert!(path.is_dir(), "every created scratch directory must exist");
        }
        paths.sort();
        paths.dedup();
        assert_eq!(
            paths.len(),
            THREADS,
            "all {THREADS} concurrently created directories must be distinct"
        );

        let all_paths: Vec<PathBuf> = dirs.iter().map(|d| d.to_path_buf()).collect();
        drop(dirs);
        for path in &all_paths {
            assert!(
                !path.exists(),
                "Drop must remove every concurrently created scratch directory"
            );
        }
    }

    #[test]
    fn a_category_that_produces_an_invalid_path_component_propagates_the_create_dir_error() {
        // A NUL byte is invalid in a path component on every platform this
        // repo targets, so `std::fs::create_dir` fails predictably and
        // portably here, giving `ScratchDir::new` a deterministic `Err` to
        // propagate without needing filesystem permission tricks or a
        // raced collision.
        let category = "bad\0category";
        let result = ScratchDir::new(category);
        assert!(
            result.is_err(),
            "a NUL byte in the category should make create_dir fail"
        );

        // No directory should have been left behind under any name derived
        // from this category -- `create_dir` failing means nothing was
        // created at all, but assert the specific prefix is absent too, so
        // a future implementation change that silently swallowed the error
        // and created a sanitized directory anyway would also be caught.
        //
        // A sentinel entry keeps the `read_dir` scan below from ever running
        // over zero entries: under a freshly isolated `TMPDIR` (e.g. this
        // repository's sandboxed CI coverage job), `std::env::temp_dir()` can
        // otherwise be empty at this point, which would make the closures
        // below never execute and vacuously pass without ever comparing a
        // real entry against `prefix`.
        let _sentinel = ScratchDir::new("lib_scan_sentinel").expect("sentinel scratch dir");
        let temp_dir = std::env::temp_dir();
        let prefix = format!("pycc_{category}_");
        let leaked = std::fs::read_dir(&temp_dir)
            .expect("temp dir should be readable")
            .filter_map(|entry| entry.ok())
            .any(|entry| entry.file_name().to_string_lossy().starts_with(&prefix));
        assert!(
            !leaked,
            "a failed ScratchDir::new must not leave a directory behind"
        );
    }

    #[test]
    fn the_lock_is_held_while_the_handle_is_alive_and_released_by_drop() {
        let dir = ScratchDir::new("lib_lock_liveness").expect("ScratchDir::new should succeed");
        let lock_path = dir.join(LOCK_FILE_NAME);
        assert!(
            lock_path.is_file(),
            "ScratchDir::new must create the liveness-marker lock file"
        );
        // A second fd on the same file gets its own open file description,
        // so this probe observes the handle's exclusive lock even from the
        // same process (verified against flock/LockFileEx semantics).
        let probe = File::open(&lock_path).expect("the lock file should be openable read-only");
        assert!(
            probe.try_lock().is_err(),
            "a probe must not acquire the lock while the creating handle is alive"
        );
        // The release half needs the unlinked-but-open probe fd to stay
        // usable after `Drop` removed the file, which unix guarantees and
        // Windows delete-pending semantics do not; the coverage platform
        // (macOS) runs it.
        #[cfg(unix)]
        {
            drop(dir);
            assert!(
                probe.try_lock().is_ok(),
                "dropping the handle must release the advisory lock"
            );
        }
    }

    #[test]
    fn a_failing_lock_file_creation_removes_the_directory_and_propagates_the_error() {
        // A lock-file name under a nonexistent subdirectory makes the
        // marker creation fail with NotFound on every target, after
        // `create_dir` already succeeded — exercising both the error
        // propagation and the just-created directory's cleanup.
        let result =
            ScratchDir::new_with_lock_file_name("lib_lock_create_fail", "missing_subdir/lock");
        assert!(
            result.is_err(),
            "a failing lock-file creation must propagate its io::Error"
        );

        // A sentinel entry keeps the `read_dir` scan below from ever running
        // over zero entries: under a freshly isolated `TMPDIR` (e.g. this
        // repository's sandboxed CI coverage job, or any run with
        // `--test-threads=1`), `std::env::temp_dir()` can otherwise be empty
        // at this point, which would make the closures below never execute
        // and vacuously pass without ever comparing a real entry against
        // `prefix`.
        let _sentinel = ScratchDir::new("lib_lock_scan_sentinel").expect("sentinel scratch dir");
        let temp_dir = std::env::temp_dir();
        let prefix = "pycc_lib_lock_create_fail_";
        let leaked = std::fs::read_dir(&temp_dir)
            .expect("temp dir should be readable")
            .filter_map(|entry| entry.ok())
            .any(|entry| entry.file_name().to_string_lossy().starts_with(prefix));
        assert!(
            !leaked,
            "a failed lock-file creation must remove the just-created directory"
        );
    }
}
