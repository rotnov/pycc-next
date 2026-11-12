//! Pass 3's pruning step (docs/design/compile-little.md, section 2.3): a
//! winning set can hold members that make the workload slower, carried by
//! the others' gain. pydantic's 9-function hot core won at -4.33% of the
//! training workload's instructions while removing `GenerateJsonSchema.mode`,
//! `GenerateJsonSchema._config` or `create_schema_validator` each made it
//! run fewer (-4.59%, -4.59%, -4.48%): each by less than the 0.5% a removal
//! must save, together by more.
//!
//! So the winner is pruned in two steps. Each member, the one with the
//! least self time first, is removed in turn and the set without it
//! measured as a whole against the winner (leave one out); the members
//! whose removal did better at all ([`lighter`]) are then removed together
//! and that set measured once more. The selection becomes the set without
//! them only when it beats the winner by the rule a larger set must meet to
//! replace a smaller one ([`prunes`]); else a single removal that does, the
//! first measured; else the winner as it was. A member that pays stays,
//! and the selection is still exactly one set measured as a whole that won.

use super::verdict::WALL_MARGIN_PPM;
use super::{lighter, measured, prunes, Check, FunctionKey, Pass, MIN_GAIN_PPM};
use std::collections::BTreeSet;

impl Pass<'_, '_> {
    /// Prunes measurement `best` within `budget` builds more: one build per
    /// member tried (`order` comes most self time first, and is tried from
    /// its end), one of the budget reserved for the members removed
    /// together. The members of the carried set are never tried, and the
    /// set is never pruned to nothing. The measurement selected, or None
    /// when nothing won. A refusal that invalidates the winner ends the
    /// step, and the best valid measurement is selected instead.
    pub(super) fn prune(
        &mut self,
        best: Option<usize>,
        order: &[FunctionKey],
        budget: usize,
    ) -> Result<Option<usize>, String> {
        let Some(winner) = best else {
            return Ok(None);
        };
        let limit = self.builds + budget;
        let base = self.base();
        let with: BTreeSet<FunctionKey> = self.entries[winner].set.clone();
        let Some(kept) = self.entries[winner].check.clone() else {
            return Ok(best);
        };
        // Least self time first: the least evidence that a member pays. The
        // last member of its own is never tried: a set is not pruned to
        // nothing.
        let tries: Vec<FunctionKey> = order
            .iter()
            .rev()
            .filter(|key| with.contains(*key) && !base.contains(*key))
            .cloned()
            .collect();
        if tries.len() < 2 || budget == 0 {
            return Ok(best);
        }
        // Leave one out, within all but the build reserved for the group.
        let mut trials: Vec<(FunctionKey, usize)> = Vec::new();
        let reserved = limit.saturating_sub(1).max(self.builds + 1);
        for key in &tries {
            if self.builds >= reserved {
                break;
            }
            let mut without = with.clone();
            without.remove(key);
            let trial = self.measure(without, reserved)?;
            if !self.entries[winner].valid {
                return Ok(self.best());
            }
            let Some(trial) = trial else {
                continue;
            };
            // Selectable only if the step picks it below.
            self.entries[trial].valid = false;
            if let Some(pruned) = self.entries[trial].check.clone() {
                let verdict = if lighter(&pruned, &kept) {
                    "better without it"
                } else {
                    "not better without it"
                };
                report(&format!("{}:{}", key.0, key.1), &pruned, &kept, verdict);
                trials.push((key.clone(), trial));
            }
        }
        let won = |pass: &Self, i: usize| {
            pass.entries[i]
                .check
                .as_ref()
                .is_some_and(|c| prunes(c, &kept))
        };
        let losers: Vec<FunctionKey> = trials
            .iter()
            .filter(|(_, i)| {
                self.entries[*i]
                    .check
                    .as_ref()
                    .is_some_and(|c| lighter(c, &kept))
            })
            .map(|(key, _)| key.clone())
            .collect();
        let mut selected = None;
        if losers.len() > 1 && losers.len() < tries.len() {
            let group: BTreeSet<FunctionKey> = with
                .iter()
                .filter(|key| !losers.contains(key))
                .cloned()
                .collect();
            let trial = self.measure(group, limit)?;
            if !self.entries[winner].valid {
                return Ok(self.best());
            }
            if let Some(trial) = trial {
                self.entries[trial].valid = false;
                if let Some(pruned) = self.entries[trial].check.clone() {
                    let names: Vec<String> = losers
                        .iter()
                        .map(|key| format!("{}:{}", key.0, key.1))
                        .collect();
                    let dropped = won(self, trial);
                    report(
                        &names.join(", "),
                        &pruned,
                        &kept,
                        if dropped { "dropped together" } else { "kept" },
                    );
                    if dropped {
                        selected = Some((trial, losers.clone()));
                    }
                }
            }
        }
        if selected.is_none() {
            selected = trials
                .iter()
                .find(|(_, i)| won(self, *i))
                .map(|(key, i)| (*i, vec![key.clone()]));
        }
        let Some((trial, dropped)) = selected else {
            eprintln!("pycc profile: pass 3: no removal beats the winning set: it is kept whole");
            return Ok(best);
        };
        self.entries[trial].valid = true;
        for key in dropped {
            eprintln!(
                "pycc profile: pass 3: {}:{} dropped from the winning set",
                key.0, key.1
            );
            // Its verdict is the winner it was removed from.
            self.verdicts
                .insert(key.clone(), measured(&kept, true, with.len()));
            self.last.insert(key, winner);
        }
        Ok(Some(trial))
    }
}

/// Says what removing `what` (one member, or several together) from the
/// winner did, with `verdict`: for one member, whether it did better at
/// all; for the group, whether the removal is kept.
fn report(what: &str, pruned: &Check, kept: &Check, verdict: &str) {
    let ms = |ns: u64| ns as f64 / 1e6;
    let rule = match (pruned.instructions, kept.instructions) {
        (Some(_), Some(_)) => format!(
            "a removal is kept when it saves {:.1}% of the workload more and does not run slower",
            MIN_GAIN_PPM as f64 / 1e4
        ),
        _ => format!(
            "a removal is kept when it is faster beyond the noise by {:.0}% of CPython's time",
            WALL_MARGIN_PPM as f64 / 1e4
        ),
    };
    match (pruned.instructions, kept.instructions) {
        (Some(p), Some(k)) => {
            let percent = |delta: i64| delta as f64 * 100.0 / p.cpython.max(1) as f64;
            eprintln!(
                "pycc profile: pass 3: without {what} the workload ran {:+.2}% instructions against {:+.2}% with: {verdict} ({rule})",
                percent(-p.saved()),
                percent(-k.saved()),
            );
        }
        _ => eprintln!(
            "pycc profile: pass 3: without {what} the workload took {:.1} ms (IQR {:.1}-{:.1}) against {:.1} ms (IQR {:.1}-{:.1}) with: {verdict} ({rule})",
            ms(pruned.compiled[1]),
            ms(pruned.compiled[0]),
            ms(pruned.compiled[2]),
            ms(kept.compiled[1]),
            ms(kept.compiled[0]),
            ms(kept.compiled[2]),
        ),
    }
}

#[cfg(test)]
#[path = "prune_tests.rs"]
mod tests;
