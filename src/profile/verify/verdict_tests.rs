use super::*;

/// A check with CPython at 100 ms (IQR 90-110) and 1,000,000 instructions,
/// compiled at `wall` and `compiled` instructions.
fn counted(wall: [u64; 3], compiled: u64) -> Check {
    Check {
        cpython: [90, 100, 110],
        compiled: wall,
        instructions: Some(Instructions { cpython: 1_000_000, compiled }),
    }
}

/// Counted, a set wins when it saves at least 0.5% of the workload's
/// instructions, however its wall times fall inside the noise; a smaller
/// saving, or more instructions, is not a win.
#[test]
fn counted_a_set_wins_by_the_instructions_it_saves() {
    // Wall times that alone would not be a win (median inside CPython's IQR).
    let noisy = [95, 105, 115];
    assert!(counted(noisy, 995_000).faster());
    assert!(counted(noisy, 800_000).faster());
    assert!(!counted(noisy, 995_001).faster());
    assert!(!counted(noisy, 1_000_000).faster());
    assert!(!counted(noisy, 1_200_000).faster());
    // Wall times that alone would be a win do not make a tie one.
    assert!(!counted([50, 60, 70], 999_000).faster());
}

/// Fewer instructions can still run slower (cache misses): wall times that
/// show the set slower beyond the noise, its first quartile over CPython's
/// third, veto the win.
#[test]
fn wall_times_slower_beyond_the_noise_veto_an_instruction_win() {
    assert!(counted([105, 115, 125], 900_000).faster());
    assert!(counted([110, 120, 130], 900_000).faster());
    assert!(!counted([111, 120, 130], 900_000).faster());
    assert!(counted([111, 120, 130], 900_000).slower());
}

/// Counted, a larger winning set replaces a smaller one when it saves at
/// least 0.5% of the workload's instructions more, whatever the wall times
/// say: the same answer on every run.
#[test]
fn counted_a_larger_set_replaces_a_smaller_one_by_the_instructions_it_saves_more() {
    let small = counted([60, 70, 80], 900_000);
    assert!(replaces(&counted([60, 70, 80], 895_000), 30, &small, 1));
    assert!(!replaces(&counted([60, 70, 80], 895_001), 30, &small, 1));
    assert!(!replaces(&counted([60, 70, 80], 910_000), 30, &small, 1));
    // Wall times far apart do not order sets the counts call even.
    assert!(!replaces(&counted([10, 20, 30], 899_000), 30, &small, 1));
    assert!(replaces(&counted([90, 99, 105], 850_000), 30, &small, 1));
}

/// A set counted against one measured on wall time alone (a carried
/// winner from an earlier wall-time profile) is compared on wall time.
#[test]
fn a_counted_set_against_a_wall_time_one_is_compared_on_wall_time() {
    let wall = Check { cpython: [90, 100, 110], compiled: [70, 75, 80], instructions: None };
    assert!(!replaces(&counted([70, 75, 80], 100), 30, &wall, 1));
    assert!(replaces(&counted([50, 55, 60], 999_999), 30, &wall, 1));
}

/// On wall time a larger set replaces a smaller one only beyond the noise
/// of both and saving 5% of CPython's median more. diff-match-patch's
/// 30-function region once came apart from `diff_bisect` alone by chance
/// (both over 5x CPython's speed): that is not a replacement.
#[test]
fn on_wall_time_a_larger_set_must_also_save_five_percent_of_cpython_s_time_more() {
    let wall = |q1, median, q3| Check { cpython: [190, 200, 210], compiled: [q1, median, q3], instructions: None };
    let small = wall(98, 100, 102);
    // Apart, but 5 ms (2.5% of CPython's 200) faster: not enough.
    assert!(!replaces(&wall(93, 95, 97), 30, &small, 1));
    assert!(replaces(&wall(85, 90, 97), 30, &small, 1));
    let dmp = |q1, median, q3| Check { cpython: [14099, 14212, 14344], compiled: [q1, median, q3], instructions: None };
    assert!(!replaces(&dmp(2237, 2535, 2575), 30, &dmp(2596, 2710, 2747), 1));
}

/// A removal from a winning set is kept by the rule a larger set must meet:
/// counted, the pruned set saves 0.5% of the workload's instructions more
/// and still wins (its wall-time veto included); on wall time it is faster
/// beyond the noise of both by 5% of CPython's median.
#[test]
fn a_removal_must_save_as_much_more_as_a_larger_set_must() {
    let current = counted([60, 70, 80], 952_000);
    // pydantic's `mode`: compiled it adds 0.55% of the workload.
    assert!(prunes(&counted([60, 70, 80], 946_500), &current));
    assert!(prunes(&counted([60, 70, 80], 947_000), &current));
    // Removing a loser worth less than 0.5%, or a winner: kept.
    assert!(!prunes(&counted([60, 70, 80], 947_001), &current));
    assert!(!prunes(&counted([60, 70, 80], 955_000), &current));
    // Fewer instructions but slower than CPython beyond the noise: vetoed.
    assert!(!prunes(&counted([111, 120, 130], 900_000), &current));
    // On wall time: apart from the current set and 5 ms (5% of 100) faster.
    let wall = |q1, median, q3| Check { cpython: [90, 100, 110], compiled: [q1, median, q3], instructions: None };
    let current = wall(75, 80, 85);
    assert!(prunes(&wall(65, 70, 74), &current));
    assert!(!prunes(&wall(65, 70, 75), &current));
    assert!(!prunes(&wall(72, 76, 74), &current));
    // Faster than the current set but not than CPython: never a win.
    assert!(!prunes(&wall(60, 95, 74), &wall(95, 100, 105)));
}

/// `auto` counts instructions for a workload CPython runs in at most a
/// second; the other signals do what they say.
#[test]
fn auto_counts_instructions_for_a_short_workload_only() {
    assert!(counts_instructions(Signal::Auto, AUTO_MAX_WALL_NS));
    assert!(!counts_instructions(Signal::Auto, AUTO_MAX_WALL_NS + 1));
    assert!(counts_instructions(Signal::Instructions, u64::MAX));
    assert!(!counts_instructions(Signal::Wall, 0));
}
