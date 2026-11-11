//! Pass 3's verdicts (docs/design/compile-little.md, section 2.3): whether a
//! check's set won, and which of two winning sets is the better one.
//!
//! A check is decided on the least noisy signal pass 3 has. Wall times of the
//! training workload in fresh processes vary by about 15% (interquartile range
//! over median) on one machine, and pairing the interleaved runs does not
//! cancel it (the noise is each process's own, not drift), so a set that
//! saves a few percent of the workload is a coin toss on wall time. Its
//! instruction count under callgrind is the same on every run, to a few
//! instructions in billions. So when pass 3 counts instructions
//! ([`Signal`]), the count decides: a set wins when the workload runs at
//! least [`MIN_GAIN_PPM`] fewer instructions with it compiled, and a larger
//! set replaces a smaller one only when it saves that much more again. The
//! wall times still veto a set they show slower beyond the noise
//! ([`Check::slower`]), since fewer instructions can run slower (cache
//! misses). Without counts, the wall times decide as before: the compiled
//! median under CPython's first quartile, and a larger set only beyond the
//! noise of both and when it saves [`WALL_MARGIN_PPM`] of CPython's time
//! more.

/// Parts per million.
pub(crate) const PPM: u64 = 1_000_000;

/// The least share of the training workload's instructions a set must save
/// to win, and a larger set must save beyond a smaller one's to replace it:
/// a gain smaller than this is not worth the code size and the risk of a
/// compiled function.
pub(crate) const MIN_GAIN_PPM: u64 = 5_000;

/// On wall time, the share of CPython's median time a larger winning set
/// must save beyond a smaller one's (median against median), besides being
/// faster beyond the noise of both, to replace it. Interquartile ranges of
/// a few rounds come apart by chance: diff-match-patch's 30-function region
/// (2535 ms, IQR 2237-2575) once replaced `diff_bisect` alone (2710 ms, IQR
/// 2596-2747) so, against CPython's 14212 ms, where 5% is 711 ms.
pub(crate) const WALL_MARGIN_PPM: u64 = 50_000;

/// The most rounds per check.
pub(crate) const MAX_ROUNDS: u32 = 15;
/// Rounds after which a check whose interquartile ranges are apart stops.
pub(crate) const EARLY_ROUNDS: u32 = 5;

/// What decides pass 3's checks (`--verify-signal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Signal {
    /// Instruction counts when valgrind runs the workload and CPython runs
    /// it in at most [`AUTO_MAX_WALL_NS`]; else wall time.
    Auto,
    /// Instruction counts under callgrind; an error when they cannot be had.
    Instructions,
    /// Wall time only.
    Wall,
}

/// Under `--verify-signal auto`, the longest CPython run of the workload
/// (its median over [`PROBE_RUNS`]) for which pass 3 counts instructions:
/// callgrind runs it about 60 to 90 times slower, once per check.
pub(crate) const AUTO_MAX_WALL_NS: u64 = 1_000_000_000;

/// The CPython runs of the workload that estimate its length for
/// [`Signal::Auto`].
pub(crate) const PROBE_RUNS: usize = 3;

/// Whether a workload whose CPython runs took `wall_ns` (their median) is
/// short enough for [`Signal::Auto`] to count its instructions.
pub(crate) fn counts_instructions(signal: Signal, wall_ns: u64) -> bool {
    match signal {
        Signal::Auto => wall_ns <= AUTO_MAX_WALL_NS,
        Signal::Instructions => true,
        Signal::Wall => false,
    }
}

/// The training workload's instructions, once in CPython and once with a
/// check's set compiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Instructions {
    pub(crate) cpython: u64,
    pub(crate) compiled: u64,
}

impl Instructions {
    /// The instructions compiling the set saved (negative: added).
    pub(crate) fn saved(&self) -> i64 {
        self.cpython as i64 - self.compiled as i64
    }

    /// Whether `saved` instructions are at least [`MIN_GAIN_PPM`] of
    /// CPython's count.
    fn enough(&self, saved: i64) -> bool {
        saved > 0 && saved as u128 * PPM as u128 >= MIN_GAIN_PPM as u128 * self.cpython as u128
    }
}

/// The outcome of one check.
#[derive(Debug, Clone)]
pub(crate) struct Check {
    /// The workload's wall time in CPython: first quartile, median, third
    /// quartile, in nanoseconds.
    pub(crate) cpython: [u64; 3],
    /// The same with the set compiled.
    pub(crate) compiled: [u64; 3],
    /// Its instructions, when pass 3 counted them.
    pub(crate) instructions: Option<Instructions>,
}

impl Check {
    /// Whether the set won. Counted: the workload ran at least
    /// [`MIN_GAIN_PPM`] fewer instructions compiled, and its wall times do
    /// not show it slower ([`Check::slower`]). Else the compiled median
    /// under CPython's first quartile, outside the noise of CPython's own
    /// runs.
    pub(crate) fn faster(&self) -> bool {
        match self.instructions {
            Some(i) => i.enough(i.saved()) && !self.slower(),
            None => self.compiled[1] < self.cpython[0],
        }
    }

    /// Whether the wall times show the set slower beyond the noise: the
    /// compiled first quartile over CPython's third.
    pub(crate) fn slower(&self) -> bool {
        self.compiled[0] > self.cpython[2]
    }
}

/// First quartile, median and third quartile (each the median of its half).
pub(crate) fn quartiles(samples: &[u64]) -> [u64; 3] {
    let mut v = samples.to_vec();
    v.sort_unstable();
    let median = |v: &[u64]| -> u64 {
        match v.len() {
            0 => 0,
            n if n % 2 == 1 => v[n / 2],
            n => (v[n / 2 - 1] + v[n / 2]) / 2,
        }
    };
    let n = v.len();
    if n < 2 {
        let m = median(&v);
        return [m, m, m];
    }
    [median(&v[..n / 2]), median(&v), median(&v[n.div_ceil(2)..])]
}

/// Whether the two samples' interquartile ranges do not overlap: the wall
/// verdict will not change with more rounds.
pub(crate) fn clear(plain: &[u64], built: &[u64]) -> bool {
    let (p, b) = (quartiles(plain), quartiles(built));
    b[0] > p[2] || b[2] < p[0]
}

/// Whether a winning set of `larger` functions checked `challenger` replaces
/// one of `smaller` functions checked `incumbent` (`larger >= smaller`).
/// Each function more costs build time, size and risk, so the larger set
/// must earn its place by more than noise. Both counted: it saves at least
/// [`MIN_GAIN_PPM`] of the workload's instructions more, which gives the
/// same answer on every run. Else on wall time: its compiled third quartile
/// under the incumbent's first, and its median under the incumbent's by at
/// least [`WALL_MARGIN_PPM`] of the incumbent's CPython median; two medians
/// whose interquartile ranges overlap do not order the sets, as the pick
/// between them would flip from run to run. A set of the same size
/// replaces the one measured first (more self time) the same way.
pub(crate) fn replaces(challenger: &Check, larger: usize, incumbent: &Check, smaller: usize) -> bool {
    debug_assert!(larger >= smaller);
    match (challenger.instructions, incumbent.instructions) {
        (Some(c), Some(i)) => c.enough(c.saved() - i.saved()),
        _ => {
            let saved_more = incumbent.compiled[1].saturating_sub(challenger.compiled[1]);
            challenger.compiled[2] < incumbent.compiled[0]
                && saved_more as u128 * PPM as u128 >= incumbent.cpython[1] as u128 * WALL_MARGIN_PPM as u128
        }
    }
}

#[cfg(test)]
#[path = "verdict_tests.rs"]
mod tests;
