//! How many threads one `pycc` process's own parallel work may use.
//!
//! A build alone on a machine may use every core: the frontend's attempt
//! waves (`crate::specialize`) run their attempts side by side. A member
//! build of a package built whole is not alone: the package build runs
//! `PYCC_JOBS` members at a time, one per core by default, so the cores are
//! already busy and a member's extra threads only compete with the other
//! members for them. The package build gives each member its share
//! ([`member_share`], passed as `--member-threads`), and work that is
//! parallel only to save time -- not because it is needed -- is not done
//! when the share is one thread (`crate::specialize::with_specialization`'s
//! speculative attempt).

use std::sync::OnceLock;

static BUDGET: OnceLock<usize> = OnceLock::new();

/// Limits this process to `threads` (at least one). Set once, before any
/// parallel work starts; a second call is ignored.
pub(crate) fn set_budget(threads: usize) {
    let _ = BUDGET.set(threads.max(1));
}

/// The threads this process may use: the budget set, else every core.
pub(crate) fn budget() -> usize {
    BUDGET
        .get()
        .copied()
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from))
}

/// The threads each of `jobs` member builds running at once on `cores`
/// cores may use: an equal share, at least one.
pub(crate) fn member_share(cores: usize, jobs: usize) -> usize {
    (cores / jobs.max(1)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_gets_an_equal_share_of_the_cores_and_at_least_one() {
        assert_eq!(member_share(4, 4), 1);
        assert_eq!(member_share(4, 8), 1);
        assert_eq!(member_share(8, 2), 4);
        assert_eq!(member_share(4, 1), 4);
        assert_eq!(member_share(3, 2), 1);
        assert_eq!(member_share(1, 0), 1);
    }

    #[test]
    fn the_budget_is_at_least_one_thread() {
        assert!(budget() >= 1);
    }
}
