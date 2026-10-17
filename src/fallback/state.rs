//! The fallback loop's per-thread state: the rewritten entry source the
//! loader reads, and the functions of that rewrite twinned in CPython.
//! Split out of `fallback.rs`; [`Snapshot`] carries it to the threads the
//! specialization loop runs attempts on (`crate::specialize`).

use super::Rendered;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) static REPORT: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The rewritten entry source the loader reads instead of the file.
    static ENTRY_OVERRIDE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Records `--report-fallbacks`.
pub(crate) fn set_report(report: bool) {
    REPORT.store(report, Ordering::Relaxed);
}

/// Whether `--report-fallbacks` was given.
pub(crate) fn reporting() -> bool {
    REPORT.load(Ordering::Relaxed)
}

/// Whether the program being built has fallback code.
pub(crate) fn active() -> bool {
    ENTRY_OVERRIDE.with(|cell| cell.borrow().is_some())
}

/// The rewritten entry source, when a fallback plan is active.
pub(crate) fn entry_override() -> Option<String> {
    ENTRY_OVERRIDE.with(|cell| cell.borrow().clone())
}

/// The entry module's source as the frontend compiles it: the fallback
/// rewrite when one is active, else the file itself.
pub(crate) fn entry_source(path: &Path) -> String {
    entry_override().unwrap_or_else(|| std::fs::read_to_string(path).unwrap_or_default())
}

pub(super) fn set_override(text: Option<String>) {
    if text.is_none() {
        TWINNED.with(|cell| cell.borrow_mut().clear());
        ATTACHED.with(|cell| cell.borrow_mut().clear());
    }
    ENTRY_OVERRIDE.with(|cell| *cell.borrow_mut() = text);
}

thread_local! {
    /// The functions of the active rewrite that also run in CPython for
    /// Python callers (`Plan::twinned`): not exported compiled.
    static TWINNED: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

thread_local! {
    /// The methods of the active rewrite attached to compiled classes
    /// (`Rendered::attached`).
    static ATTACHED: RefCell<Vec<Attached>> = const { RefCell::new(Vec::new()) };
}

/// A class with methods attached to it, where its statement starts in the
/// rewrite, and the attributes of `self` those methods use.
pub(crate) type Attached = (String, u32, BTreeSet<String>);

/// Makes `rendered` the entry source the frontend compiles.
pub(super) fn install(rendered: &Rendered) {
    set_override(Some(rendered.text.clone()));
    TWINNED.with(|cell| *cell.borrow_mut() = rendered.twinned.clone());
    ATTACHED.with(|cell| *cell.borrow_mut() = rendered.attached.clone());
}

/// The active rewrite's attached methods (`Rendered::attached`).
pub(crate) fn attached_methods() -> Vec<Attached> {
    ATTACHED.with(|cell| cell.borrow().clone())
}

/// The compiled functions (and their `pycc_ast::ext_signatures` entries)
/// an `--ext` module does not export, because Python code calls their
/// CPython twin instead (`Plan::twinned`).
pub(crate) fn twinned_exports() -> BTreeSet<String> {
    TWINNED.with(|cell| {
        cell.borrow()
            .iter()
            .flat_map(|name| [name.clone(), pycc_ast::ext_entry_name(name)])
            .collect()
    })
}

/// This thread's fallback state, to install on another thread
/// ([`Snapshot::install`]) so an attempt there compiles the same rewrite.
#[derive(Clone)]
pub(crate) struct Snapshot {
    entry: Option<String>,
    twinned: BTreeSet<String>,
    attached: Vec<Attached>,
}

impl Snapshot {
    /// The current thread's state.
    pub(crate) fn take() -> Self {
        Self {
            entry: entry_override(),
            twinned: TWINNED.with(|cell| cell.borrow().clone()),
            attached: attached_methods(),
        }
    }

    /// Makes `self` the current thread's state.
    pub(crate) fn install(self) {
        ENTRY_OVERRIDE.with(|cell| *cell.borrow_mut() = self.entry);
        TWINNED.with(|cell| *cell.borrow_mut() = self.twinned);
        ATTACHED.with(|cell| *cell.borrow_mut() = self.attached);
    }
}
