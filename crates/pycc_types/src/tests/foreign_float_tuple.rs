//! Part 4 of #1026 (PR 4c of #1083): the one annotated assignment that
//! admits a CPython object.
//!
//! `x: tuple[float, float, float] = <object>` in a module body compiles,
//! and so does any fixed-arity annotation whose every element is `float`.
//! `foreign::is_object_float_tuple_annotation` owns the rule; these tests
//! pin it from the outside, in both directions, on the one arm that
//! consults it.
//!
//! **Why the relaxation is a branch and not a widening.** It sits in
//! `check_stmt`'s module-level `HirStmt::AnnAssign` arm, ahead of
//! `is_assignable_env`. `is_assignable` is consulted wherever a value flows
//! into a declared type -- a parameter, a `return`, a class attribute -- so
//! widening it would have admitted the pair at every one of those too,
//! which is not what Part 4 supports. The negative tests below are the
//! statement of that: the same pair at any other position stays refused.

use super::*;

/// Lowers `source` with the driver answering `ResolvedImport::Foreign` for
/// its leading `import gc`, then type-checks the result.
///
/// `lower_checked` cannot serve here: it lowers against an empty answer
/// table, where `import gc` is an ordinary unsupported stdlib import
/// (`C0001`) rather than a foreign binding. Resolving the import is the
/// driver's job (`src/modules.rs`), so the answer is supplied directly --
/// `pycc_hir`'s own `program/tests.rs` establishes the pattern.
fn lower_foreign(source: &str) -> Result<pycc_hir::HirModule, Vec<pycc_diag::Diagnostic>> {
    let parsed = pycc_parser::parse(source).expect("test fixture must parse");
    let mut resolved = pycc_hir::ResolvedImports::default();
    // The key is whatever span `pycc_hir` asks under (an alias's own span
    // for a plain `import` since #1280), so it is taken from the request
    // rather than re-derived here.
    let request = pycc_hir::project_import_requests(&parsed)
        .into_iter()
        .find(|request| request.module.as_deref() == Some("gc") && request.names.is_empty())
        .expect("the fixture must contain its `import gc`");
    resolved.insert(request.span, pycc_hir::ResolvedImport::Foreign);
    pycc_hir::lower_module(&parsed, &resolved, None).map(|lowered| lowered.hir)
}

fn check_source(source: &str) -> Result<(), Vec<pycc_diag::Diagnostic>> {
    check_all(&lower_foreign(source).expect("test fixture must lower"))
}

fn first_code(source: &str) -> String {
    let diagnostics = check_source(source).unwrap_err();
    diagnostics[0].code.to_string()
}

#[test]
fn a_module_level_float_tuple_annotation_admits_a_cpython_object() {
    // Every arity from one upward, and both foreign producer shapes a
    // module body can reach -- the bare name and an attribute load -- since
    // the rule keys on the initializer's *type*, never on the expression
    // that produced it (`foreign.rs`'s module documentation).
    for initializer in ["gc", "gc.garbage"] {
        for arity in [1usize, 2, 3, 5] {
            let annotation = std::iter::repeat_n("float", arity)
                .collect::<Vec<_>>()
                .join(", ");
            let source = format!("import gc\n\nx: tuple[{annotation}] = {initializer}\n");
            assert!(
                check_source(&source).is_ok(),
                "{source}: {:?}",
                check_source(&source).unwrap_err()
            );
        }
    }
}

#[test]
fn the_bound_name_carries_the_annotated_tuple_type() {
    // The admission would be worthless if the name stayed opaque: the
    // point of the unpack is that `x` is an ordinary pycc tuple afterwards.
    // Subscripting it with a literal index and using the element as a
    // `float` is what proves the binding took the *annotation's* type
    // rather than `Ty::Object` -- an object-typed `x` would make `x[0]`
    // an `I0404` and the arithmetic a `T0021`.
    assert!(
        check_source(
            "import gc\n\nx: tuple[float, float, float] = gc.garbage\ny: float = x[0] + 1.0\nprint(y)\n"
        )
        .is_ok()
    );
}

#[test]
fn a_mixed_tuple_annotation_keeps_its_unchanged_refusal() {
    // The `elems.iter().all(...)` clause, from the outside. `T0025` is the
    // ordinary annotated-assignment mismatch this arm already produced --
    // the branch is skipped and the pre-existing rejection runs unchanged.
    for annotation in ["float, int", "int, float", "float, float, bool"] {
        let source = format!("import gc\n\nx: tuple[{annotation}] = gc.garbage\n");
        assert_eq!(first_code(&source), "T0025", "{source}");
    }
}

#[test]
fn a_non_tuple_annotation_keeps_its_unchanged_refusal() {
    // The `matches!(ty, Ty::Tuple(_))` clause. A `list[int]` is the
    // nearest miss worth pinning: it is the other fixed-element-type
    // container spelling this compiler admits, and it is not a by-value
    // aggregate, so there is no unpack to emit for it.
    for annotation in ["float", "int", "list[int]", "str"] {
        let source = format!("import gc\n\nx: {annotation} = gc.garbage\n");
        assert_eq!(first_code(&source), "T0025", "{source}");
    }
}

#[test]
fn a_variadic_tuple_annotation_is_refused_earlier_still() {
    // PEP 585's `tuple[float, ...]` never reaches this crate: `pycc_hir`
    // refuses the `...` type argument with `T0053` while lowering the
    // annotation, because a homogeneous-variadic container has no
    // compile-time length and D-115 holds a tuple by value at a fixed
    // width. Asserted at the lowering layer rather than through
    // `check_source`, which would panic on the `lower_checked` expect.
    //
    // The Part 4 plan predicted `T0025` here. The outcome is the same --
    // the shape stays refused -- but the diagnostic is `T0053` and it is
    // produced one layer earlier, so this test records what the tree
    // actually does rather than what the plan assumed.
    let diagnostics = lower_foreign("import gc\n\nx: tuple[float, ...] = gc.garbage\n")
        .expect_err("the `...` type argument is refused");
    assert_eq!(diagnostics[0].code, "T0053", "{diagnostics:?}");
}

#[test]
fn the_admitted_pair_stays_refused_at_every_other_declared_position() {
    // The reason the relaxation is not a widening of `is_assignable`.
    // Each of these flows a `Ty::Object` into a declared
    // `tuple[float, float, float]` somewhere that is *not* a module-level
    // annotated assignment, and each must stay refused.
    for (code, source) in [
        // A `return` under a declared return type. `T0022` rather than
        // `I0404`: #1316 admits the in-function read of the module-level
        // foreign `gc`, and the declared-return check reports the
        // `object`-versus-tuple mismatch before the `return` refusal of a
        // CPython object. Either way the relaxation never reaches a
        // `return`.
        (
            "T0022",
            "import gc\n\ndef f() -> tuple[float, float, float]:\n    return gc.garbage\n",
        ),
        // The same statement inside a function body, which is exactly why
        // the in-function `AnnAssign` arm needs no branch of its own: #1316
        // admits the read, and the ordinary assignability check refuses
        // the `object` value against the declared tuple (`T0025`).
        (
            "T0025",
            "import gc\n\ndef f() -> None:\n    x: tuple[float, float, float] = gc.garbage\n",
        ),
    ] {
        assert_eq!(first_code(source), code, "{source}");
    }
}

#[test]
fn a_module_mixing_a_generic_function_with_the_unpack_checks_clean() {
    // The #1104/#1105 defect class: a module that mixes a generic function
    // with foreign-object code exercises the monomorphization pass, whose
    // HIR-walking arms are where a new shape gets silently dropped.
    //
    // 4c adds **no HIR variant** -- the HIR is an ordinary `AnnAssign` over
    // an ordinary value expression -- so `monomorphize.rs`'s
    // `rewrite_protocol_calls_in_stmt`, `bind_local_types_in_stmt` and the
    // empty-container pre-pass each already have the arm this shape needs.
    // This test is the statement of that, at the layer where it is decidable.
    //
    // It deliberately stops at `check_all` rather than at `pycc build
    // --ext`. #1105 is open and reports that *any* module mixing a generic
    // function with foreign-object code exits 0 under `pycc check` and 1
    // under `pycc build --ext` with a spurious `T0021`. That defect is
    // pre-existing and unrelated to this change; asserting the build here
    // would pin #1105's bug rather than 4c's behaviour.
    assert!(
        check_source(
            "import gc\n\
             \n\
             def identity[T](value: T) -> T:\n\
             \x20   return value\n\
             \n\
             x: tuple[float, float, float] = gc.garbage\n\
             print(identity(1))\n"
        )
        .is_ok()
    );
}

#[test]
fn a_final_float_tuple_declaration_is_admitted_too() {
    // The `is_final` tail of the arm is shared rather than duplicated into
    // the new branch, so a `Final` 4c declaration must behave like any
    // other: admitted once, and rejected on a second assignment with the
    // PEP 591 `T0045`.
    assert!(check_source("import gc\n\nx: Final[tuple[float, float]] = gc.garbage\n").is_ok());
    assert_eq!(
        first_code("import gc\n\nx: Final[tuple[float, float]] = gc.garbage\nx = (1.0, 2.0)\n"),
        "T0045"
    );
}
