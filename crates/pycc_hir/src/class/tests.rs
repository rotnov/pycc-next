use crate::{HirClassDef, HirExpr, HirItem, HirStmt, Ty, lower_checked};

pub(super) fn assert_c0001(source: &str) {
    let module = crate::pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001", "source: {source:?}");
}

pub(super) fn lower_ok(source: &str) -> crate::HirModule {
    // `.expect(...)`, not `.unwrap_or_else(|e| panic!(...))`: the
    // latter's closure body is its own hand-written region, never
    // executed on this helper's own happy path (every call site
    // expects success) -- this crate's own established coverage-gate
    // convention (see `init_slot::slot_ty_from_init_rhs`'s own doc
    // comment) is `.expect()`, whose panic path
    // lives in libcore, outside this crate's instrumented regions.
    let module = crate::pycc_parser_test_helper::parse(source);
    lower_checked(&module).expect("test fixture should lower successfully")
}

// Topic submodules split out of this file.
mod declarations;
mod methods;
mod properties_and_static;
mod inheritance;
mod dataclasses;
mod protocols_and_abc;
