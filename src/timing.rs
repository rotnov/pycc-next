//! `PYCC_TIME=1`: per-phase wall-clock timings of one `pycc` invocation,
//! printed to stderr when the command ends (`docs/CLI_SPEC.md`).
//!
//! Each [`span`] adds its duration to the phase it names; a phase entered
//! several times (one frontend attempt per fallback or specialization
//! round) is reported with its count. Nested spans each count their full
//! duration, so the rows do not sum to the total. With the variable unset
//! a span costs one environment lookup, done once per process.

use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Phase name -> (times entered, total duration), in first-entered order.
static PHASES: Mutex<Vec<(&'static str, u32, Duration)>> = Mutex::new(Vec::new());

/// Whether `PYCC_TIME` is set (to anything but `0` or the empty string).
pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var_os("PYCC_TIME").is_some_and(|v| !v.is_empty() && v != "0"))
}

/// Runs `f`, adding its wall-clock duration to `phase` when timing is on.
pub(crate) fn span<T>(phase: &'static str, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let start = Instant::now();
    let value = f();
    record(phase, start.elapsed());
    value
}

fn record(phase: &'static str, elapsed: Duration) {
    let mut phases = PHASES.lock().unwrap_or_else(|e| e.into_inner());
    match phases.iter_mut().find(|(name, _, _)| *name == phase) {
        Some((_, count, total)) => {
            *count += 1;
            *total += elapsed;
        }
        None => phases.push((phase, 1, elapsed)),
    }
}

/// The report [`print`] writes: one line per phase.
pub(crate) fn render(total: Duration) -> String {
    let phases = PHASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = String::new();
    for (name, count, elapsed) in phases.iter() {
        out.push_str(&format!(
            "pycc-time: {name:<28} {:>9.3}s  x{count}\n",
            elapsed.as_secs_f64()
        ));
    }
    out.push_str(&format!(
        "pycc-time: {:<28} {:>9.3}s\n",
        "total",
        total.as_secs_f64()
    ));
    out
}

/// Prints the report to stderr when timing is on.
pub(crate) fn print(started: Instant) {
    if enabled() {
        eprint!("{}", render(started.elapsed()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phase_entered_twice_is_one_row_with_its_count() {
        record("test-phase-a", Duration::from_millis(5));
        record("test-phase-a", Duration::from_millis(7));
        let report = render(Duration::from_millis(20));
        let row = report
            .lines()
            .find(|line| line.contains("test-phase-a"))
            .unwrap();
        assert!(row.ends_with("x2"), "{row}");
        assert!(row.contains("0.012s"), "{row}");
        assert!(report.lines().last().unwrap().contains("total"));
    }

    #[test]
    fn span_returns_the_value_of_its_closure() {
        assert_eq!(span("test-phase-b", || 41 + 1), 42);
    }
}
