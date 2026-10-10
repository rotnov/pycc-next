//! Unit tests for the crate-root lowering surface.
//!
//! Extracted verbatim from `lib.rs`'s inline `mod tests` (issue #547): the
//! module is still a direct child of the crate root, so `use super::*` and the
//! private items it reaches resolve exactly as they did inline.

use super::*;
use crate::expr::keyword_bind::SignatureTable;
use std::collections::HashSet;
// `lower_comprehension_header`/`rename_name_in_expr` moved to `expr.rs`
// (issue #361, D-149) but the two tests below call them directly,
// bypassing the public `lower_checked` entry point -- unlike every other
// test in this module, so a `super::*` glob import alone does not reach
// them (the crate root neither defines nor re-exports them). See
// `expr.rs`'s own module doc comment for why these two stayed here
// instead of moving.
use crate::expr::{lower_comprehension_header, rename_name_in_expr};

// Subscripted-annotation lowering (`ClassName[...]`, PEP 560 #611/#693,
// D-228 containers, #931 non-class bases) lives in its own child module so
// this file does not keep growing (#663). A child module sees this module's
// private helpers through `use super::*`.
mod subscript_annotations;

// The pre-PEP 585 `typing` container aliases (#1378), same reason.
mod typing_legacy_aliases;

// Comprehensions in expression position (#1254, D-250).
mod comprehension_expr;

// `ClassVar` in a `@dataclass` body (#913, D-235) likewise lives in its own
// child module for the same reason.
mod dataclass_class_vars;

// Class attributes named after the instantiation/class-creation protocol
// (#975, D-236), same reason again.
mod reserved_dunder_class_attrs;

// `Enum`-body assignments CPython's `_EnumDict` keeps out of the member list
// (#979, D-238) -- a different name set and a route-gated rule, so its own
// module rather than more of the one above.
mod enum_non_member_names;

// A class body binding `__setattr__` or `__delattr__` (#1459) -- the
// attribute-store protocol set, checked on the method and attribute routes.
mod store_protocol_names;

// A class body binding `__getattribute__` or `__getattr__` (#1465) -- the
// attribute-read protocol set, the read-side sibling of the one above.
mod read_protocol_names;

// The `C0001` that makes `--ext`'s abstract-method exclusion total
// (#1145) -- a derivation nothing pinned before, and its discriminating
// counterpart. Its own module for the reason the five above give.
mod abstract_without_abc_base;

// D-228's bare-container advice positions (#918), moved out when #1266 added
// the class-body declaration position to the advising set.
mod bare_container_advice;

// Part 2a of #1371: which `X[Y](args)` calls stay a generic class
// instantiation and which lower to a call of a subscript result.
mod subscript_call;

// Topic submodules split out of this file.
mod control_flow;
mod ty_and_errors;
mod basics;
mod functions;
mod containers;
mod comprehensions;
mod slices_and_rename;
mod aliases_and_imports;
mod generics_super_isinstance;
mod match_stmt;
mod flow_analysis;
mod capability_messages;
mod container_annotations;

fn assert_capability_error(source: &str, expected_message: &str, expected_span: Span) {
    let module = pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
    assert!(diagnostic.message.contains(expected_message));
    assert_eq!(diagnostic.span, Some(expected_span));
}

fn assert_capability_error_message(source: &str, expected_message: &str) {
    let module = pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
    assert!(diagnostic.message.contains(expected_message));
    assert!(diagnostic.span.is_some());
}
