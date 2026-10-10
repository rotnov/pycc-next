//! Constraint-collection unit tests for the type-checking crate root.
//!
//! Extracted verbatim from `tests.rs` under AGENTS.md's decomposability rule
//! (part of #695, which tracks decomposing that oversized file). These are the
//! tests that exercise the constraint-collection pass in the production sibling
//! `crate::constraints`, so the split mirrors the source layout. As a child
//! module this still sees the parent's private items directly through
//! `use super::*`, so nothing needed widened visibility; only the tests'
//! location changed.

use super::*;

// Topic submodules split out of this file.
mod bindings;
mod lists;
mod builtin_calls;
mod statements;
mod comprehensions;
