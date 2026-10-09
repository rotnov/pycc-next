//! Part 2 of #1175 (#1179): the two independent computations of the
//! buffer-sub-range out-slot fact must agree.
//!
//! The driver decides it here, from HIR, when it builds an
//! [`ExtExport`][super::ExtExport] -- that answer shapes the generated C's
//! `extern` declaration, its cast and its argument list. Codegen decides it
//! again, from MIR, when it widens the compiled function's LLVM signature and
//! when it emits the export thunk. Neither can see the other's input.
//!
//! A disagreement is not a diagnostic. The generated C would call a function
//! through a prototype with the wrong arity -- three missing or three
//! surplus pointer arguments -- which neither compiler can detect across the
//! object boundary and which manifests as a corrupted stack or a SIGBUS at
//! run time. So the agreement is pinned here by a corpus that covers all
//! three buffer-return provenances at once, rather than left to the two
//! walks' structural similarity.

use super::*;

/// `return <name>[1:3]`.
fn slice_return(name: &str) -> pycc_hir::HirStmt {
    pycc_hir::HirStmt::Return(Some(pycc_hir::HirExpr::Slice {
        base: Box::new(pycc_hir::HirExpr::Name(name.to_string())),
        start: Some(Box::new(pycc_hir::HirExpr::IntLiteral(1))),
        stop: Some(Box::new(pycc_hir::HirExpr::IntLiteral(3))),
        step: None,
    }))
}

/// `return <name>`.
fn name_return(name: &str) -> pycc_hir::HirStmt {
    pycc_hir::HirStmt::Return(Some(pycc_hir::HirExpr::Name(name.to_string())))
}

/// Codegen's own answer for the function named `name`, reached the way the
/// real build reaches it: lower the same HIR to MIR, then ask
/// `pycc_codegen::buffer_slice_out_names` which *names* carry out-slots.
///
/// The name, not one definition of it. Every `def` of a name shares one
/// `fnptr_<name>` slot and one LLVM signature, so codegen resolves the fact
/// per name over every definition and the driver unions the same way at its
/// dedup site. A probe that asked the first -- or the last -- definition
/// would answer a question neither side asks, and would have passed while
/// the artifact mismatched.
fn codegen_answer(hir: &pycc_hir::HirModule, name: &str) -> bool {
    let mir = pycc_mir::build(hir);
    assert!(
        mir.items.iter().any(|item| matches!(
            item,
            pycc_mir::MirItem::Function { name: item_name, .. } if item_name == name
        )),
        "`{name}` should be lowered"
    );
    pycc_codegen::buffer_slice_out_names(&mir.items).contains(name)
}

/// The corpus: every buffer-return provenance, plus a non-buffer export, in
/// one module -- so the two walks also have to agree about *which* of
/// several exports carries out-slots, not merely about whether any does.
#[test]
fn the_driver_and_codegen_agree_on_every_provenance() {
    let hir = module(vec![
        // Part 1 of #1175: the whole caller-owned view. No bounds, so no
        // out-slots, and its generated C must stay byte-identical.
        func_with_body(
            "whole",
            &[("b", Ty::MemoryView)],
            Ty::MemoryView,
            vec![name_return("b")],
        ),
        // Part 2b of #1142: artifact-owned storage. Also no bounds -- and
        // #1179 deliberately does not widen this provenance, so a slice of
        // it stays refused rather than reaching either walk.
        func_with_body(
            "owned",
            &[("n", Ty::Int)],
            Ty::MemoryView,
            vec![
                pycc_hir::HirStmt::Assign {
                    target: "a".to_string(),
                    value: pycc_hir::HirExpr::Call {
                        callee: "ndarray".to_string(),
                        args: vec![pycc_hir::HirExpr::Name("n".to_string())],
                    },
                },
                name_return("a"),
            ],
        ),
        // Part 2 of #1175: the sub-range.
        func_with_body(
            "sliced",
            &[("b", Ty::MemoryView)],
            Ty::MemoryView,
            vec![slice_return("b")],
        ),
        // Both provenances inside one export, on separate branches, with
        // the slice nested rather than at the top level.
        func_with_body(
            "mixed",
            &[("b", Ty::MemoryView), ("c", Ty::MemoryView)],
            Ty::MemoryView,
            vec![pycc_hir::HirStmt::If {
                test: pycc_hir::HirExpr::BoolLiteral(true),
                body: vec![name_return("b")],
                orelse: vec![slice_return("c")],
            }],
        ),
        // No buffer at all.
        func("plain", &[("x", Ty::Int)], Ty::Int),
    ]);

    let exports = collect_exports(&hir).expect("the corpus exports cleanly");
    let driver: Vec<(&str, bool)> = exports
        .iter()
        .map(|export| (export.name.as_str(), export.returns_buffer_slice))
        .collect();
    assert_eq!(
        driver,
        vec![
            ("whole", false),
            ("owned", false),
            ("sliced", true),
            ("mixed", true),
            ("plain", false),
        ],
        "the driver's own answer, from HIR"
    );

    for (name, expected) in &driver {
        assert_eq!(
            codegen_answer(&hir, name),
            *expected,
            "codegen disagrees with the driver about `{name}`"
        );
    }
}

/// The driver's answer keys on the buffer *parameter* a slice is taken of,
/// not merely on a slice being present: slicing a `str` parameter inside a
/// buffer-returning export carries no out-slots.
#[test]
fn a_slice_of_a_non_buffer_parameter_carries_no_out_slots() {
    let hir = module(vec![func_with_body(
        "f",
        &[("b", Ty::MemoryView), ("s", Ty::Str)],
        Ty::MemoryView,
        vec![
            pycc_hir::HirStmt::Assign {
                target: "head".to_string(),
                value: pycc_hir::HirExpr::Slice {
                    base: Box::new(pycc_hir::HirExpr::Name("s".to_string())),
                    start: None,
                    stop: Some(Box::new(pycc_hir::HirExpr::IntLiteral(1))),
                    step: None,
                },
            },
            name_return("b"),
        ],
    )]);
    let exports = collect_exports(&hir).expect("exports cleanly");
    assert!(!exports[0].returns_buffer_slice);
    assert!(!codegen_answer(&hir, "f"));
}

/// The fact is per-export-body, so two buffer parameters are distinguished:
/// returning a slice of the *second* one is still an out-slot-carrying
/// export, and returning a slice of neither is not.
#[test]
fn either_buffer_parameter_may_be_the_sliced_one() {
    for sliced in ["b", "c"] {
        let hir = module(vec![func_with_body(
            "f",
            &[("b", Ty::MemoryView), ("c", Ty::MemoryView)],
            Ty::MemoryView,
            vec![slice_return(sliced)],
        )]);
        let exports = collect_exports(&hir).expect("exports cleanly");
        assert!(exports[0].returns_buffer_slice, "{sliced}");
        assert!(codegen_answer(&hir, "f"), "{sliced}");
    }
}

/// Two `def`s of one name, in both orderings.
///
/// The arity is a property of the shared `fnptr_<name>` slot, so both
/// definitions are widened and both sides must say so -- including when the
/// *last* definition, the one actually bound, only does a bare `return b`.
/// Resolving this last-wins instead is what made the generated C declare
/// one arity while the compiled function carried another: an ill-typed call
/// across the object boundary that neither compiler can see, and, in the
/// other ordering, a thunk that forwarded four arguments through a
/// one-argument `fn_type` and aborted the compiler in `verify()`.
#[test]
fn a_redefined_name_carries_out_slots_in_both_orderings() {
    for (label, first, second) in [
        ("bare_then_sliced", name_return("b"), slice_return("b")),
        ("sliced_then_bare", slice_return("b"), name_return("b")),
    ] {
        let hir = module(vec![
            func_with_body("f", &[("b", Ty::MemoryView)], Ty::MemoryView, vec![first]),
            func_with_body("f", &[("b", Ty::MemoryView)], Ty::MemoryView, vec![second]),
        ]);
        let exports = collect_exports(&hir).expect("the redefinition exports cleanly");
        assert_eq!(exports.len(), 1, "{label}: one wrapper per C function");
        assert!(
            exports[0].returns_buffer_slice,
            "{label}: the driver must widen the shared signature"
        );
        assert!(
            codegen_answer(&hir, "f"),
            "{label}: codegen must widen the same name"
        );
    }
}

/// ...and a name redefined without any definition slicing stays un-widened,
/// so the union does not simply answer "true" whenever two `def`s share a
/// name.
#[test]
fn a_redefined_name_with_no_slice_carries_no_out_slots() {
    let hir = module(vec![
        func_with_body(
            "f",
            &[("b", Ty::MemoryView)],
            Ty::MemoryView,
            vec![name_return("b")],
        ),
        func_with_body(
            "f",
            &[("b", Ty::MemoryView)],
            Ty::MemoryView,
            vec![name_return("b")],
        ),
    ]);
    let exports = collect_exports(&hir).expect("the redefinition exports cleanly");
    assert_eq!(exports.len(), 1);
    assert!(!exports[0].returns_buffer_slice);
    assert!(!codegen_answer(&hir, "f"));
}
