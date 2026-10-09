//! Bounded, best-effort removal of stale pycc scratch roots (issue #784,
//! Part 4 of #779).
//!
//! A pycc process that dies without running `Drop` (SIGKILL, power loss, a
//! `kill -9`'d test runner) leaves its scratch root behind. This module is
//! the defense-in-depth janitor: [`sweep_stale_roots`] scans the OS temp
//! directory and deletes roots whose creating process is provably dead,
//! under strict budgets so the caller's own latency stays bounded.
//!
//! # Safety bar
//!
//! The sweep deletes an entry only if it
//!
//! (a) fully parses as `pycc_{category}_{pid}_{nanos}_{seq}` with a
//!     non-empty category (the [D-201] ownership marker — parsed numbers
//!     are validation only, never trusted as timestamps),
//! (b) is a real directory (not a symlink — `DirEntry::file_type` does not
//!     follow symlinks, and `remove_dir_all` on a symlink-to-dir would only
//!     remove the link anyway),
//! (c) exceeds the age floor for its class ([`SweepConfig::min_age_locked`]
//!     for roots carrying a [`LOCK_FILE_NAME`] marker,
//!     [`SweepConfig::min_age_lockless`] for roots without an observable
//!     one — pre-Part-4 legacy roots, or a Part-4 root whose creator was
//!     killed before its lock file was created), and
//! (d) holds no live lock: a successful [`File::try_lock`] on the marker is
//!     kernel-backed proof the creator is dead, immune to PID reuse and
//!     released even on SIGKILL.
//!
//! The floors measure the directory's mtime, which tracks entry churn
//! (create/remove/rename inside the root), not last IO — good enough for
//! their purpose (covering the create-to-lock window, mid-`Drop` races, and
//! the marker-less population: pre-Part-4 legacy roots, or a Part-4 root
//! killed before its lock file was created), and stated plainly rather than
//! glossed as "activity". Every fallible step folds conservatively into a
//! skip; a concurrent sweep winning a deletion race is tolerated
//! (`NotFound` from `remove_dir_all` counts as already-deleted).
//!
//! One platform divergence, safe on both sides: a *directory* sitting at
//! the lock path (which no pycc process ever creates) opens fine on unix —
//! where `try_lock` on a directory fd acquires, so the root reads as dead
//! and is deleted once past the floor — while on Windows the open fails
//! non-`NotFound` and the root is conservatively skipped as an error.
//!
//! [D-201]: https://github.com/rotnov/pycc/blob/main/docs/decisions/D-201-shared-pycc-scratch-crate-and-lint-gate-for.md

use std::fs::File;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use crate::LOCK_FILE_NAME;

/// Budgets and age floors for one sweep pass. Every field has a measured
/// rationale (see the D-209 decision entry): the defaults keep a worst-case
/// pass around the [`SweepConfig::time_budget`] cap and a typical pass on a
/// clean temp directory in the low milliseconds.
#[derive(Debug, Clone)]
pub(crate) struct SweepConfig {
    /// Maximum directory entries examined before the pass stops
    /// (~1.9 µs/entry measured, so the default scans in ~20 ms).
    pub entry_budget: usize,
    /// Maximum roots deleted (deletion outcomes counted, including
    /// already-deleted races) before the pass stops.
    pub deletion_budget: usize,
    /// Hard wall-clock cap, checked between entries (a single pathological
    /// `remove_dir_all` can still overshoot it — accepted; typical pycc
    /// roots hold one object file or executable).
    pub time_budget: Duration,
    /// Minimum age for roots that carry a [`LOCK_FILE_NAME`] marker. The
    /// lock is the liveness source; this floor only covers the microsecond
    /// create-dir-to-lock window and mid-`Drop` races.
    pub min_age_locked: Duration,
    /// Minimum age for roots without an observable marker, whose liveness
    /// cannot be probed: pre-Part-4 legacy roots, or a Part-4 root whose
    /// creator was killed between `create_dir` and lock-file creation.
    pub min_age_lockless: Duration,
}

impl Default for SweepConfig {
    fn default() -> Self {
        SweepConfig {
            entry_budget: 10_000,
            deletion_budget: 512,
            time_budget: Duration::from_millis(250),
            min_age_locked: Duration::from_secs(60 * 60),
            min_age_lockless: Duration::from_secs(24 * 60 * 60),
        }
    }
}

/// What one sweep pass did. Purely informational — the production caller
/// discards it — but it is what every test asserts against.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SweepReport {
    /// Directory entries examined.
    pub scanned: usize,
    /// Entries passing the full-format ownership parse and the
    /// real-directory check.
    pub matched: usize,
    /// Roots removed (including a concurrent sweep winning the race —
    /// the root is gone, which is the outcome that was wanted).
    pub deleted: usize,
    /// Roots kept because the lock probe said the creator is alive.
    pub kept_live: usize,
    /// Roots kept because they are under the applicable age floor
    /// (future-mtime clock skew folds in here as age zero).
    pub kept_young: usize,
    /// Conservative skips: the sweep root was unreadable, a probe open
    /// failed with something other than `NotFound`, or a deletion failed.
    pub errors: usize,
    /// True when any of the three budgets stopped the scan early.
    pub budget_exhausted: bool,
}

/// Sweeps the OS temp directory with `SweepConfig::default` and the
/// current time. This is the production entry point, called (and its report
/// discarded) by `pycc build`/`pycc run` before they create their own
/// scratch root. Silent and best-effort by design: it never affects the
/// caller's output or exit code.
pub fn sweep_stale_roots() -> SweepReport {
    // `temp_dir()` is passed as the sweep root, never `.join(...)`-ed —
    // scratch roots themselves still come only from `ScratchDir::new`.
    sweep_stale_roots_in(
        &std::env::temp_dir(),
        &SweepConfig::default(),
        SystemTime::now(),
    )
}

/// The injectable seam behind [`sweep_stale_roots`]: sweeps `root` with the
/// given budgets, measuring ages against the injected `now` (letting tests
/// make hermetic fixtures "old" without touching mtimes).
///
/// Never errors: an unreadable `root` is reported as `errors: 1` in the
/// returned [`SweepReport`], matching the best-effort contract.
pub(crate) fn sweep_stale_roots_in(
    root: &Path,
    config: &SweepConfig,
    now: SystemTime,
) -> SweepReport {
    let start = Instant::now();
    let mut report = SweepReport::default();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => {
            report.errors += 1;
            return report;
        }
    };
    // `.flatten()` drops per-entry iteration errors deliberately: such an
    // `Err` is not craftable on any CI platform, so a handling arm would be
    // a permanently uncovered region; skipping the entry is the same
    // conservative outcome the explicit arm would pick.
    for entry in entries.flatten() {
        if report.scanned == config.entry_budget
            || report.deleted == config.deletion_budget
            || start.elapsed() >= config.time_budget
        {
            report.budget_exhausted = true;
            break;
        }
        report.scanned += 1;

        // Ownership gate: the full D-201 name format. `to_string_lossy`
        // rather than `to_str` — the lossy `None`-free form has no
        // uncoverable arm, and a U+FFFD replacement landing in a numeric
        // field fails the digit parse below anyway (one confined to the
        // category position parses cleanly and is accepted, consistent
        // with the full-format-name-equals-ownership rule).
        let name = entry.file_name();
        if !parses_as_scratch_root(&name.to_string_lossy()) {
            continue;
        }
        // Real directories only. `file_type` does not follow symlinks, so
        // a symlink-to-dir reports false here; an errored `file_type`
        // folds into the same skip.
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        report.matched += 1;

        // Age from mtime. A metadata/mtime read error folds to `now`
        // (age zero), and future-mtime clock skew folds to zero via
        // `duration_since`'s Err — both land in the conservative
        // `kept_young` path with no distinct region.
        let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(now);
        let age = now.duration_since(mtime).unwrap_or(Duration::ZERO);
        if age < config.min_age_locked {
            report.kept_young += 1;
            continue;
        }

        let path = entry.path();
        match File::open(path.join(LOCK_FILE_NAME)) {
            Ok(probe) => {
                // `try_lock` failing — `WouldBlock` or an io error alike —
                // reads as "cannot prove the creator dead": keep the root.
                let creator_alive = probe.try_lock().is_err();
                // Close the probe before deleting: on Windows the open
                // handle inside the tree could block the removal.
                drop(probe);
                if creator_alive {
                    report.kept_live += 1;
                } else {
                    delete_root(&path, &mut report);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // No observable marker (a pre-Part-4 legacy root, or a
                // Part-4 root killed before its lock file was created):
                // liveness cannot be probed, so the longer floor applies.
                if age >= config.min_age_lockless {
                    delete_root(&path, &mut report);
                } else {
                    report.kept_young += 1;
                }
            }
            Err(_) => {
                report.errors += 1;
            }
        }
    }
    report
}

/// Removes one root, folding the outcome into `report`: `NotFound` counts
/// as already-deleted (a concurrent sweep won the race — tolerated by
/// design), any other failure as a conservative `errors` skip.
fn delete_root(path: &Path, report: &mut SweepReport) {
    match std::fs::remove_dir_all(path) {
        Ok(()) => report.deleted += 1,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => report.deleted += 1,
        Err(_) => report.errors += 1,
    }
}

/// True when `name` fully matches `pycc_{category}_{pid}_{nanos}_{seq}`
/// with a non-empty category — the D-201 ownership format. Fields are
/// split right-to-left because `category` itself may contain `_`
/// (e.g. `codegen_test`). The numeric parses validate the shape only;
/// their values are never used (a pre-epoch clock legitimately produces
/// `nanos == 0`).
fn parses_as_scratch_root(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("pycc_") else {
        return false;
    };
    let mut fields = rest.rsplitn(4, '_');
    let (Some(seq), Some(nanos), Some(pid), Some(category)) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return false;
    };
    seq.parse::<u64>().is_ok()
        && nanos.parse::<u128>().is_ok()
        && pid.parse::<u32>().is_ok()
        && !category.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScratchDir;
    use std::path::PathBuf;

    /// Generous budgets so no assertion-bearing test trips a budget by
    /// accident; the budget tests below override individual fields.
    fn roomy_config() -> SweepConfig {
        SweepConfig {
            entry_budget: 1_000,
            deletion_budget: 1_000,
            time_budget: Duration::from_secs(60),
            ..SweepConfig::default()
        }
    }

    /// `SystemTime::now()` shifted far enough into the future that a fresh
    /// fixture reads as older than the given number of hours.
    fn now_plus_hours(hours: u64) -> SystemTime {
        SystemTime::now() + Duration::from_secs(hours * 60 * 60)
    }

    /// Creates a full-format-named directory inside `arena`; with
    /// `with_lock_file`, also drops an *unheld* lock-marker file inside it
    /// (the shape a SIGKILLed creator leaves behind).
    fn make_root(arena: &Path, name: &str, with_lock_file: bool) -> PathBuf {
        let root = arena.join(name);
        std::fs::create_dir(&root).expect("fixture root should be creatable");
        if with_lock_file {
            File::create(root.join(LOCK_FILE_NAME)).expect("fixture lock file should be creatable");
        }
        root
    }

    #[test]
    fn the_ownership_parse_accepts_and_rejects_exactly_the_full_format() {
        let accepted = [
            "pycc_build_123_456_0",
            // `category` may itself contain underscores; `rsplitn` keeps
            // the remainder intact.
            "pycc_codegen_test_123_456_7",
            // A pre-epoch clock degrades `nanos` to 0 — still owned.
            "pycc_run_123_0_0",
            // Maximal in-range values still parse.
            "pycc_x_4294967295_340282366920938463463374607431768211455_18446744073709551615",
            // A lossy U+FFFD confined to the category position digit-parses
            // cleanly and is accepted: category is only checked non-empty.
            "pycc_c\u{FFFD}t_1_2_3",
        ];
        for name in accepted {
            assert!(
                parses_as_scratch_root(name),
                "{name:?} should parse as a pycc scratch root"
            );
        }

        let rejected = [
            // No pycc_ prefix at all.
            "cargo-install-xyz",
            // Prefix but too few fields for pid/nanos/seq.
            "pycc_notaroot",
            "pycc_a_1_2",
            // Empty category remainder.
            "pycc__1_2_3",
            // Non-numeric pid / nanos / seq (one per field, exercising each
            // short-circuit operand).
            "pycc_x_z_2_3",
            "pycc_x_1_z_3",
            "pycc_x_1_2_z",
            // Overflow: pid wider than u32, seq wider than u64.
            "pycc_x_4294967296_2_3",
            "pycc_x_1_2_18446744073709551616",
            // A lossy U+FFFD landing in a numeric field fails the digit
            // parse (the reject path non-UTF-8 names actually take).
            "pycc_x_1\u{FFFD}_2_3",
        ];
        for name in rejected {
            assert!(
                !parses_as_scratch_root(name),
                "{name:?} should not parse as a pycc scratch root"
            );
        }
    }

    #[test]
    fn a_stale_locked_format_root_past_the_floor_is_deleted() {
        let arena = ScratchDir::new("sweep_stale_locked").expect("arena should be creatable");
        let root = make_root(&arena, "pycc_stale_1_2_0", true);

        let report = sweep_stale_roots_in(&arena, &roomy_config(), now_plus_hours(2));

        assert_eq!(report.matched, 1);
        assert!(!report.budget_exhausted);
        // Windows flake guard (docs/TESTING.md): Defender contention can
        // transiently turn the deletion into an `errors` skip, so only
        // non-Windows asserts the strict outcome.
        #[cfg(not(windows))]
        {
            assert_eq!(report.deleted, 1, "the stale root must be deleted");
            assert!(!root.exists(), "the stale root must be gone");
        }
        #[cfg(windows)]
        {
            assert_eq!(
                report.deleted + report.errors,
                1,
                "the stale root must at least be claimed for deletion"
            );
            // `root`'s existence is unassertable under the contention class
            // above; keep the binding used on this side too.
            let _ = &root;
        }
    }

    #[test]
    fn a_root_whose_lock_is_held_survives_as_kept_live() {
        let arena = ScratchDir::new("sweep_kept_live").expect("arena should be creatable");
        let root = make_root(&arena, "pycc_live_1_2_0", true);
        let holder = File::options()
            .read(true)
            .write(true)
            .open(root.join(LOCK_FILE_NAME))
            .expect("the fixture lock file should be openable");
        holder
            .lock()
            .expect("the fixture lock should be acquirable");

        let report = sweep_stale_roots_in(&arena, &roomy_config(), now_plus_hours(2));

        assert_eq!(report.matched, 1);
        assert_eq!(report.kept_live, 1, "a held lock must protect the root");
        assert_eq!(report.deleted, 0);
        assert!(root.is_dir(), "the live root must survive");
        drop(holder);
    }

    #[test]
    fn a_young_root_survives_as_kept_young_before_the_probe_is_paid() {
        let arena = ScratchDir::new("sweep_kept_young").expect("arena should be creatable");
        let root = make_root(&arena, "pycc_young_1_2_0", true);

        // Real `now`: the fixture is seconds old, far under the 1 h floor.
        let report = sweep_stale_roots_in(&arena, &roomy_config(), SystemTime::now());

        assert_eq!(report.matched, 1);
        assert_eq!(report.kept_young, 1);
        assert_eq!(report.deleted, 0);
        assert!(root.is_dir(), "a young root must survive");
    }

    #[test]
    fn a_future_mtime_root_folds_into_kept_young() {
        let arena = ScratchDir::new("sweep_future_mtime").expect("arena should be creatable");
        let root = make_root(&arena, "pycc_skew_1_2_0", true);

        // Injecting a `now` *behind* the fixture's real mtime makes
        // `duration_since` fail; the fold treats the age as zero.
        let past = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        let report = sweep_stale_roots_in(&arena, &roomy_config(), past);

        assert_eq!(report.matched, 1);
        assert_eq!(report.kept_young, 1, "clock skew must read as young");
        assert!(root.is_dir(), "a future-mtime root must survive");
    }

    #[test]
    fn a_lockless_root_is_deleted_only_past_the_lockless_floor() {
        let arena = ScratchDir::new("sweep_lockless").expect("arena should be creatable");
        let root = make_root(&arena, "pycc_legacy_1_2_0", false);

        // Past the 1 h locked floor but under the 24 h lockless floor:
        // kept.
        let report = sweep_stale_roots_in(&arena, &roomy_config(), now_plus_hours(2));
        assert_eq!(report.matched, 1);
        assert_eq!(report.kept_young, 1);
        assert!(root.is_dir(), "a lockless root under 24 h must survive");

        // Past the 24 h lockless floor: deleted.
        let report = sweep_stale_roots_in(&arena, &roomy_config(), now_plus_hours(25));
        assert_eq!(report.matched, 1);
        #[cfg(not(windows))]
        {
            assert_eq!(report.deleted, 1);
            assert!(!root.exists(), "a lockless root past 24 h must be deleted");
        }
        #[cfg(windows)]
        assert_eq!(report.deleted + report.errors, 1);
    }

    #[cfg(unix)]
    #[test]
    fn an_unopenable_lock_file_is_a_conservative_errors_skip() {
        use std::os::unix::fs::PermissionsExt;

        let arena = ScratchDir::new("sweep_probe_error").expect("arena should be creatable");
        let root = make_root(&arena, "pycc_denied_1_2_0", true);
        let lock_path = root.join(LOCK_FILE_NAME);
        std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o000))
            .expect("chmod 000 should succeed");
        // uid 0 (e.g. a root container) bypasses permission checks, so the
        // mode-000 fixture cannot produce the denial there — the root then
        // legitimately reads as stale-and-dead and is deleted. Probe once
        // and *derive* the expected counts from it, branch-free: an `if`
        // on privilege would leave its never-taken edge as a permanently
        // uncovered region on the (single-uid, non-root) coverage platform.
        let privileged = File::open(&lock_path).is_ok();

        let report = sweep_stale_roots_in(&arena, &roomy_config(), now_plus_hours(2));

        assert_eq!(report.matched, 1);
        assert_eq!(
            report.errors,
            usize::from(!privileged),
            "an unopenable lock file must be a conservative skip"
        );
        assert_eq!(report.deleted, usize::from(privileged));
        assert_eq!(root.is_dir(), !privileged, "the root must survive the skip");

        // Restore permissions so the arena's own Drop can clean up. Under
        // uid 0 the root is already gone and this fails; that is fine —
        // there is nothing left to restore.
        let _ = std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o644));
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_at_the_lock_path_reads_as_dead_on_unix() {
        // Pins the documented platform divergence: on unix, opening a
        // directory read-only succeeds and `try_lock` on the dir fd
        // acquires, so such a root (which no pycc process ever creates)
        // reads as dead and is deleted once past the floor. On Windows the
        // open fails non-NotFound and the root is skipped as an error —
        // both outcomes are conservative for live processes.
        let arena = ScratchDir::new("sweep_dir_at_lock").expect("arena should be creatable");
        let root = make_root(&arena, "pycc_dirlock_1_2_0", false);
        std::fs::create_dir(root.join(LOCK_FILE_NAME))
            .expect("the dir-at-lock-path fixture should be creatable");

        let report = sweep_stale_roots_in(&arena, &roomy_config(), now_plus_hours(2));

        assert_eq!(report.matched, 1);
        assert_eq!(report.deleted, 1, "a dir at the lock path reads as dead");
        assert!(!root.exists());
    }

    #[test]
    fn non_matching_entries_are_scanned_but_never_matched() {
        let arena = ScratchDir::new("sweep_non_matching").expect("arena should be creatable");
        // A directory whose name fails the ownership parse.
        let non_matching = arena.join("pycc_notaroot");
        std::fs::create_dir(&non_matching).expect("fixture dir should be creatable");
        // A full-format-named plain *file*.
        let plain_file = arena.join("pycc_file_1_2_0");
        std::fs::write(&plain_file, b"not a directory").expect("fixture file should be writable");
        // A full-format-named symlink to a directory (unix only —
        // `file_type` does not follow it, so it must not match).
        #[cfg(unix)]
        let symlink = {
            let target = arena.join("symlink_target");
            std::fs::create_dir(&target).expect("symlink target should be creatable");
            let link = arena.join("pycc_link_1_2_0");
            std::os::unix::fs::symlink(&target, &link).expect("symlink should be creatable");
            link
        };

        let report = sweep_stale_roots_in(&arena, &roomy_config(), now_plus_hours(25));

        assert_eq!(report.matched, 0, "nothing here may match");
        assert_eq!(report.deleted, 0);
        assert!(non_matching.is_dir(), "the non-matching dir must survive");
        assert!(plain_file.is_file(), "the plain file must survive");
        #[cfg(unix)]
        assert!(
            symlink.symlink_metadata().is_ok(),
            "the symlink must survive untouched"
        );
    }

    #[test]
    fn the_entry_budget_stops_the_scan_early() {
        let arena = ScratchDir::new("sweep_entry_budget").expect("arena should be creatable");
        // Strictly more entries than the budget: the budget check runs at
        // loop-top only, so with exactly budget-many entries the loop
        // would end after the last entry without another check and
        // `budget_exhausted` would stay false.
        make_root(&arena, "pycc_a_1_2_0", true);
        make_root(&arena, "pycc_b_1_2_0", true);
        let config = SweepConfig {
            entry_budget: 1,
            ..roomy_config()
        };

        let report = sweep_stale_roots_in(&arena, &config, SystemTime::now());

        assert!(report.budget_exhausted, "the entry budget must trip");
        assert_eq!(report.scanned, 1, "only one entry may be examined");
    }

    #[test]
    fn the_deletion_budget_stops_the_scan_early() {
        let arena = ScratchDir::new("sweep_deletion_budget").expect("arena should be creatable");
        // Strictly more stale roots than the budget, for the same
        // loop-top reason as the entry-budget fixture.
        let a = make_root(&arena, "pycc_a_1_2_0", true);
        let b = make_root(&arena, "pycc_b_1_2_0", true);
        let config = SweepConfig {
            deletion_budget: 1,
            ..roomy_config()
        };

        let report = sweep_stale_roots_in(&arena, &config, now_plus_hours(2));

        // The budget trips only when a deletion actually succeeds, so the
        // Windows Defender delete-contention class (deletion counted as
        // `errors`) can keep it from tripping there at all.
        #[cfg(not(windows))]
        {
            assert!(report.budget_exhausted, "the deletion budget must trip");
            assert_eq!(report.deleted, 1, "exactly one root may be deleted");
            assert!(
                a.exists() != b.exists(),
                "exactly one of the two roots must survive"
            );
        }
        #[cfg(windows)]
        {
            assert!(report.budget_exhausted || report.errors > 0);
            let _ = (a, b);
        }
    }

    #[test]
    fn a_zero_time_budget_stops_the_scan_at_the_first_entry() {
        let arena = ScratchDir::new("sweep_time_budget").expect("arena should be creatable");
        make_root(&arena, "pycc_a_1_2_0", true);
        let config = SweepConfig {
            time_budget: Duration::ZERO,
            ..roomy_config()
        };

        let report = sweep_stale_roots_in(&arena, &config, SystemTime::now());

        assert!(report.budget_exhausted, "a zero time budget must trip");
        assert_eq!(report.scanned, 0, "no entry may be examined");
        assert_eq!(report.deleted, 0);
    }

    #[test]
    fn a_nonexistent_sweep_root_reports_one_error() {
        let arena = ScratchDir::new("sweep_missing_root").expect("arena should be creatable");
        let missing = arena.join("does_not_exist");

        let report = sweep_stale_roots_in(&missing, &roomy_config(), SystemTime::now());

        assert_eq!(
            report,
            SweepReport {
                errors: 1,
                ..SweepReport::default()
            },
            "an unreadable sweep root must be a one-error empty report"
        );
    }

    #[test]
    fn delete_root_counts_a_lost_race_as_already_deleted() {
        let arena = ScratchDir::new("sweep_delete_race").expect("arena should be creatable");
        let mut report = SweepReport::default();

        delete_root(&arena.join("never_existed"), &mut report);

        assert_eq!(
            report.deleted, 1,
            "NotFound must count as already-deleted (a concurrent sweep won)"
        );
        assert_eq!(report.errors, 0);
    }

    #[cfg(unix)]
    #[test]
    fn delete_root_counts_an_undeletable_tree_as_an_error() {
        use std::os::unix::fs::PermissionsExt;

        let arena = ScratchDir::new("sweep_delete_error").expect("arena should be creatable");
        let root = make_root(&arena, "pycc_undeletable_1_2_0", false);
        // A mode-000 subdirectory inside the root makes `remove_dir_all`
        // fail partway (it cannot read the subdirectory's entries).
        let blocker = root.join("blocker");
        std::fs::create_dir(&blocker).expect("blocker dir should be creatable");
        std::fs::create_dir(blocker.join("child")).expect("blocker child should be creatable");
        std::fs::set_permissions(&blocker, std::fs::Permissions::from_mode(0o000))
            .expect("chmod 000 should succeed");
        // uid 0 bypasses permission checks, so the mode-000 blocker cannot
        // make `remove_dir_all` fail there — the deletion then legitimately
        // succeeds. Same branch-free probe-and-derive shape as the
        // probe-error test above.
        let privileged = std::fs::read_dir(&blocker).is_ok();

        let mut report = SweepReport::default();
        delete_root(&root, &mut report);

        assert_eq!(
            report.errors,
            usize::from(!privileged),
            "an undeletable tree must count as an error"
        );
        assert_eq!(report.deleted, usize::from(privileged));

        // Restore permissions so the arena's own Drop can clean up. Under
        // uid 0 the whole root is already gone and this fails; that is
        // fine — there is nothing left to restore.
        let _ = std::fs::set_permissions(&blocker, std::fs::Permissions::from_mode(0o755));
    }

    #[test]
    fn the_production_wrapper_sweeps_the_real_temp_dir_and_returns() {
        // No count assertions: the real temp dir is shared with everything
        // else on the machine, and deleting genuinely stale past-floor
        // roots there is exactly the intended production behavior.
        let report = sweep_stale_roots();
        assert!(
            report.scanned <= SweepConfig::default().entry_budget,
            "the default entry budget must bound the scan"
        );
    }
}
