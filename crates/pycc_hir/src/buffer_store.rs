//! The single syntactic definition of *"does this function body store into
//! the buffer parameter named `name`?"* (Part 1 of #1142).
//!
//! It lives in `pycc_hir` rather than in the driver because both consumers
//! need it and neither can derive it from the other: `src/ext_build.rs`'s
//! `collect_exports` and `ctor_descriptor` ask it to decide whether the
//! generated wrapper requests `PyBUF_WRITABLE` for that parameter, and the
//! predicate has to be the *same* question `pycc_types` answers when it
//! admits the store -- a store the checker admits but this walk misses is a
//! write through storage acquired read-only, which is a memory-safety
//! defect rather than a diagnostic one.
//!
//! It is necessarily syntactic. `collect_exports` runs on the typed HIR
//! *after* checking, and `Environment` carries no ext-mode or export-set
//! flag for the checker to consult, so there is no shared typed answer to
//! reuse -- only the one HIR shape both sides see. That shape is exact
//! rather than approximate: `pycc_hir` lowers every `<bare name>[k] = v` to
//! [`HirStmt::DictSet`], whose `dict` field is the target's plain name, so
//! "the body stores into `name`" is a name comparison and nothing more.
//!
//! **Why no aliasing analysis is owed.** Every read of a buffer-bound name
//! other than `name[i]`, `len(name)` and -- since Part 1 of #1142 --
//! `name[i] = v` is still `pycc_types`' `reject_memoryview_read` `C0001`
//! (D-244's 2026-09-17 Part-1-of-#1027 amendment, statement (e)), so a
//! buffer cannot be aliased into a local, passed on, stored or returned: it
//! never leaves the slot its own name binds. A later widening of that read
//! surface would invalidate this walk, and would have to revisit it.
//!
//! **Why a module-level walk answers for a class's methods too.** An
//! exported instance, static or class method reaches the driver as a
//! module-level `HirItem::Function` under its mangled `<Class>.<method>`
//! name, so `collect_exports` hands this predicate that method's own body
//! exactly as it hands it a plain `def`'s. A `Protocol` member with a
//! buffer parameter is satisfiable under `--ext` for the same reason: the
//! implementing class's method, not the protocol's stub, is what carries
//! the body this walk reads.

use crate::{HirExpr, HirStmt};

/// Whether `body` contains an element store whose target is the name
/// `name`, at any nesting depth.
///
/// The `match` below is deliberately exhaustive with no `_` arm: a missed
/// recursion is the memory-safety failure mode this predicate exists to
/// prevent, so a future block-carrying `HirStmt` variant must be a compile
/// error here rather than a silent hole.
pub fn body_stores_into(body: &[HirStmt], name: &str) -> bool {
    body.iter().any(|stmt| stmt_stores_into(stmt, name))
}

fn stmt_stores_into(stmt: &HirStmt, name: &str) -> bool {
    match stmt {
        HirStmt::DictSet { dict, .. } => dict == name,
        HirStmt::If { body, orelse, .. } => {
            body_stores_into(body, name) || body_stores_into(orelse, name)
        }
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => body_stores_into(body, name),
        HirStmt::Match { cases, .. } => cases.iter().any(|case| body_stores_into(&case.body, name)),
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            body_stores_into(body, name)
                || handlers
                    .iter()
                    .any(|handler| body_stores_into(&handler.body, name))
                || body_stores_into(orelse, name)
                || body_stores_into(finalbody, name)
        }
        // Every remaining statement carries no nested block, so it can
        // contain no store: a `DictSet` is a statement and never a
        // sub-expression, and no expression position can hold one.
        HirStmt::ExprStmt(_)
        | HirStmt::Assign { .. }
        | HirStmt::AnnAssign { .. }
        | HirStmt::ListCompAssign { .. }
        | HirStmt::DictCompAssign { .. }
        | HirStmt::SetCompAssign { .. }
        | HirStmt::Return(_)
        | HirStmt::AttrSet { .. }
        | HirStmt::Delete { .. }
        | HirStmt::DeleteSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::ForeignImport { .. }
        | HirStmt::Raise { .. } => false,
    }
}

/// Whether `body` returns a step-free slice of `name`, at any nesting
/// depth (Part 2 of #1175, #1179).
///
/// The driver-side half of the per-export "this body carries a buffer
/// sub-range egress" fact `src/ext_build.rs` stores on `ExtExport` and
/// `crates/pycc_codegen/src/ext.rs` recomputes from MIR. The shape is
/// exactly the one `pycc_types::buffer::admitted_buffer_return` admits --
/// `return <name>[start:stop]`, a bare-name base and no `step` -- so for a
/// program that type-checked, this predicate and the codegen-side walk for
/// `MirStmt::ReturnBufferSlice` answer the same question about the same
/// function. A parity test pins them together rather than leaving the
/// agreement to review.
///
/// Syntactic and total, like [`body_stores_into`] above: the caller
/// supplies the provenance half by asking only about names it already knows
/// to be `memoryview` parameters.
#[must_use]
pub fn body_returns_slice_of(body: &[HirStmt], name: &str) -> bool {
    body.iter().any(|stmt| stmt_returns_slice_of(stmt, name))
}

fn stmt_returns_slice_of(stmt: &HirStmt, name: &str) -> bool {
    match stmt {
        HirStmt::Return(Some(HirExpr::Slice {
            base, step: None, ..
        })) => matches!(base.as_ref(), HirExpr::Name(base_name) if base_name == name),
        HirStmt::If { body, orelse, .. } => {
            body_returns_slice_of(body, name) || body_returns_slice_of(orelse, name)
        }
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => body_returns_slice_of(body, name),
        HirStmt::Match { cases, .. } => cases
            .iter()
            .any(|case| body_returns_slice_of(&case.body, name)),
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            body_returns_slice_of(body, name)
                || handlers
                    .iter()
                    .any(|handler| body_returns_slice_of(&handler.body, name))
                || body_returns_slice_of(orelse, name)
                || body_returns_slice_of(finalbody, name)
        }
        // Every remaining statement either carries no nested block or is a
        // `return` of some other shape; a slice egress is only ever the
        // whole operand of a `return`, never a sub-expression, because
        // `pycc_types` refuses a buffer slice in every other position.
        HirStmt::Return(_)
        | HirStmt::ExprStmt(_)
        | HirStmt::Assign { .. }
        | HirStmt::AnnAssign { .. }
        | HirStmt::ListCompAssign { .. }
        | HirStmt::DictCompAssign { .. }
        | HirStmt::SetCompAssign { .. }
        | HirStmt::DictSet { .. }
        | HirStmt::AttrSet { .. }
        | HirStmt::Delete { .. }
        | HirStmt::DeleteSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::ForeignImport { .. }
        | HirStmt::Raise { .. } => false,
    }
}

/// Whether `body` contains a `return` statement lexically inside the
/// `finally` clause of any `try` statement, at any nesting depth
/// (Part 2b of #1142, #1164 review round 5).
///
/// This is the checked precondition of `crates/pycc_codegen/src/lib.rs`'s
/// **single** per-frame pending-return record. That record -- one pointer
/// slot plus one orphan flag -- can describe exactly one suspended return,
/// and a second `return` reached while an earlier one is still in flight
/// overwrites it. Two returns are in flight *simultaneously* only when the
/// second one is lexically inside a `finally` that the first one's own exit
/// path runs, so refusing that shape at the buffer-egress admission turns
/// "at most one return is pending at a time" from an assumption into a
/// property of every admitted program. `crates/pycc_types/src/buffer.rs`'s
/// `buffer_return_inside_finally` is the refusal, and this is its predicate.
///
/// It must be **transitive**, and the reason is a concrete host crash: the
/// reproduction that prompted it is a `return` inside a `try`/`finally`
/// inside a `while` inside a `finally` body, which a check of a `finally`
/// body's direct children misses entirely. `pycc_hir`'s own PEP 765
/// `L0001` rule (`crate::stmt`'s `in_finally`) is deliberately *not* that
/// predicate and cannot be reused as one: it follows CPython and clears
/// `in_finally` on loop entry, so `while True: return a` inside a `finally`
/// is valid Python that this walk must still see.
///
/// The `match` below is exhaustive with no `_` arm for
/// [`body_stores_into`]'s reason, and the failure mode is the same class:
/// a missed recursion re-admits a shape that hands the host a `memoryview`
/// over freed storage.
///
/// There is no arm for a nested function definition because [`HirStmt`] has
/// no such variant -- a `def` inside a `def` never reaches this
/// representation -- so "do not descend into nested functions" is vacuous
/// here rather than enforced.
pub fn body_returns_inside_finally(body: &[HirStmt]) -> bool {
    body.iter().any(stmt_returns_inside_finally)
}

fn stmt_returns_inside_finally(stmt: &HirStmt) -> bool {
    match stmt {
        // The one arm that answers the question rather than recursing: a
        // `finally` clause is entered, so *any* `return` anywhere beneath it
        // counts. The other three parts of the same `try` are ordinary
        // nested blocks and keep recursing.
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            body_contains_return(finalbody)
                || body_returns_inside_finally(body)
                || handlers
                    .iter()
                    .any(|handler| body_returns_inside_finally(&handler.body))
                || body_returns_inside_finally(orelse)
                || body_returns_inside_finally(finalbody)
        }
        HirStmt::If { body, orelse, .. } => {
            body_returns_inside_finally(body) || body_returns_inside_finally(orelse)
        }
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => body_returns_inside_finally(body),
        HirStmt::Match { cases, .. } => cases
            .iter()
            .any(|case| body_returns_inside_finally(&case.body)),
        // No nested block, so no `finally` can begin here.
        HirStmt::ExprStmt(_)
        | HirStmt::Assign { .. }
        | HirStmt::AnnAssign { .. }
        | HirStmt::DictSet { .. }
        | HirStmt::ListCompAssign { .. }
        | HirStmt::DictCompAssign { .. }
        | HirStmt::SetCompAssign { .. }
        | HirStmt::Return(_)
        | HirStmt::AttrSet { .. }
        | HirStmt::Delete { .. }
        | HirStmt::DeleteSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::ForeignImport { .. }
        | HirStmt::Raise { .. } => false,
    }
}

/// Whether `body` contains a `return` statement at any nesting depth.
///
/// Called only on a `finally` clause's own statements, where the enclosing
/// `finally` has already been entered, so every nested block -- including a
/// nested `try`'s own `finally` -- is searched uniformly and the
/// [`HirStmt::Try`] arm needs no special case.
fn body_contains_return(body: &[HirStmt]) -> bool {
    body.iter().any(stmt_contains_return)
}

fn stmt_contains_return(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::Return(_) => true,
        HirStmt::If { body, orelse, .. } => {
            body_contains_return(body) || body_contains_return(orelse)
        }
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => body_contains_return(body),
        HirStmt::Match { cases, .. } => cases.iter().any(|case| body_contains_return(&case.body)),
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            body_contains_return(body)
                || handlers
                    .iter()
                    .any(|handler| body_contains_return(&handler.body))
                || body_contains_return(orelse)
                || body_contains_return(finalbody)
        }
        HirStmt::ExprStmt(_)
        | HirStmt::Assign { .. }
        | HirStmt::AnnAssign { .. }
        | HirStmt::DictSet { .. }
        | HirStmt::ListCompAssign { .. }
        | HirStmt::DictCompAssign { .. }
        | HirStmt::SetCompAssign { .. }
        | HirStmt::AttrSet { .. }
        | HirStmt::Delete { .. }
        | HirStmt::DeleteSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::ForeignImport { .. }
        | HirStmt::Raise { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HirExceptHandler, HirExpr, HirMatchCase, HirPattern};

    /// `b[0] = 1.0`, the only statement shape that can answer `true`.
    fn store(name: &str) -> HirStmt {
        HirStmt::DictSet {
            dict: name.to_string(),
            key: HirExpr::IntLiteral(0),
            value: HirExpr::FloatLiteral(1.0),
        }
    }

    /// A statement carrying no nested block, so the leaf arm answers it.
    fn leaf() -> HirStmt {
        HirStmt::Return(None)
    }

    #[test]
    fn a_top_level_store_is_found_and_another_name_is_not() {
        assert!(body_stores_into(&[store("b")], "b"));
        assert!(!body_stores_into(&[store("other")], "b"));
        assert!(!body_stores_into(&[leaf()], "b"));
        assert!(!body_stores_into(&[], "b"));
    }

    /// Every block-carrying variant, each with the store in one nested
    /// position: a variant this walk failed to recurse into would let the
    /// wrapper acquire read-only storage the compiled body then writes.
    #[test]
    fn a_store_is_found_inside_every_nested_block() {
        let nested: Vec<Vec<HirStmt>> = vec![
            vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![store("b")],
                orelse: vec![],
            }],
            vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![],
                orelse: vec![store("b")],
            }],
            vec![HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: vec![store("b")],
            }],
            vec![HirStmt::ForRange {
                var: "i".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(1),
                step: HirExpr::IntLiteral(1),
                body: vec![store("b")],
            }],
            vec![HirStmt::ForList {
                var: "i".to_string(),
                list: "xs".to_string(),
                body: vec![store("b")],
            }],
            vec![HirStmt::ForObject {
                var: "i".to_string(),
                iter: Box::new(HirExpr::IntLiteral(0)),
                body: vec![store("b")],
            }],
            vec![HirStmt::Match {
                subject: HirExpr::IntLiteral(0),
                cases: vec![HirMatchCase {
                    pattern: HirPattern::Wildcard,
                    guard: None,
                    body: vec![store("b")],
                }],
            }],
            vec![try_stmt(vec![store("b")], vec![], vec![], vec![])],
            vec![try_stmt(
                vec![],
                vec![HirExceptHandler {
                    exc_type: None,
                    name: None,
                    body: vec![store("b")],
                }],
                vec![],
                vec![],
            )],
            vec![try_stmt(vec![], vec![], vec![store("b")], vec![])],
            vec![try_stmt(vec![], vec![], vec![], vec![store("b")])],
            vec![HirStmt::TryStar {
                body: vec![store("b")],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![],
            }],
            // Two levels deep, so the recursion is not merely one-deep.
            vec![HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::If {
                    test: HirExpr::BoolLiteral(true),
                    body: vec![store("b")],
                    orelse: vec![],
                }],
            }],
        ];
        for (index, body) in nested.iter().enumerate() {
            assert!(body_stores_into(body, "b"), "shape {index}: {body:?}");
            assert!(!body_stores_into(body, "c"), "shape {index}: {body:?}");
        }
    }

    /// Every nested block a `return` can hide in *below* a `finally`, so a
    /// variant the finally-side walk failed to recurse into would re-admit a
    /// buffer egress that hands the host freed storage.
    #[test]
    fn a_return_is_found_inside_every_block_nested_under_a_finally() {
        let hidden: Vec<Vec<HirStmt>> = vec![
            vec![HirStmt::Return(None)],
            vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::Return(None)],
                orelse: vec![],
            }],
            vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![],
                orelse: vec![HirStmt::Return(None)],
            }],
            vec![HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::Return(None)],
            }],
            vec![HirStmt::ForRange {
                var: "i".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(1),
                step: HirExpr::IntLiteral(1),
                body: vec![HirStmt::Return(None)],
            }],
            vec![HirStmt::ForList {
                var: "i".to_string(),
                list: "xs".to_string(),
                body: vec![HirStmt::Return(None)],
            }],
            vec![HirStmt::ForObject {
                var: "i".to_string(),
                iter: Box::new(HirExpr::IntLiteral(0)),
                body: vec![HirStmt::Return(None)],
            }],
            vec![HirStmt::Match {
                subject: HirExpr::IntLiteral(0),
                cases: vec![HirMatchCase {
                    pattern: HirPattern::Wildcard,
                    guard: None,
                    body: vec![HirStmt::Return(None)],
                }],
            }],
            vec![try_stmt(
                vec![HirStmt::Return(None)],
                vec![],
                vec![],
                vec![],
            )],
            vec![try_stmt(
                vec![],
                vec![HirExceptHandler {
                    exc_type: None,
                    name: None,
                    body: vec![HirStmt::Return(None)],
                }],
                vec![],
                vec![],
            )],
            vec![try_stmt(
                vec![],
                vec![],
                vec![HirStmt::Return(None)],
                vec![],
            )],
            vec![try_stmt(
                vec![],
                vec![],
                vec![],
                vec![HirStmt::Return(None)],
            )],
            vec![HirStmt::TryStar {
                body: vec![HirStmt::Return(None)],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![],
            }],
        ];
        for (index, finalbody) in hidden.iter().enumerate() {
            let body = vec![try_stmt(vec![], vec![], vec![], finalbody.clone())];
            assert!(
                body_returns_inside_finally(&body),
                "shape {index}: {body:?}"
            );
        }
        assert!(!body_returns_inside_finally(&[try_stmt(
            vec![],
            vec![],
            vec![],
            vec![store("b"), leaf_without_return()],
        )]));
    }

    /// The other half: a `try` whose `finally` is clean, with the `return`
    /// in each of the *other* three parts and in every block around the
    /// `try` itself. None of these puts two returns in flight, so admitting
    /// them is the point -- an over-broad predicate would refuse the shape
    /// that still ships.
    #[test]
    fn a_return_outside_every_finally_is_not_reported() {
        let clean: Vec<Vec<HirStmt>> = vec![
            vec![HirStmt::Return(None)],
            vec![try_stmt(
                vec![HirStmt::Return(None)],
                vec![],
                vec![],
                vec![],
            )],
            vec![try_stmt(
                vec![],
                vec![HirExceptHandler {
                    exc_type: None,
                    name: None,
                    body: vec![HirStmt::Return(None)],
                }],
                vec![],
                vec![],
            )],
            vec![try_stmt(
                vec![],
                vec![],
                vec![HirStmt::Return(None)],
                vec![],
            )],
            vec![HirStmt::TryStar {
                body: vec![HirStmt::Return(None)],
                handlers: vec![HirExceptHandler {
                    exc_type: None,
                    name: None,
                    body: vec![HirStmt::Return(None)],
                }],
                orelse: vec![HirStmt::Return(None)],
                finalbody: vec![store("b")],
            }],
            vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::Return(None)],
                orelse: vec![HirStmt::Return(None)],
            }],
            vec![HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::Return(None)],
            }],
            vec![HirStmt::ForRange {
                var: "i".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(1),
                step: HirExpr::IntLiteral(1),
                body: vec![HirStmt::Return(None)],
            }],
            vec![HirStmt::ForList {
                var: "i".to_string(),
                list: "xs".to_string(),
                body: vec![HirStmt::Return(None)],
            }],
            vec![HirStmt::ForObject {
                var: "i".to_string(),
                iter: Box::new(HirExpr::IntLiteral(0)),
                body: vec![HirStmt::Return(None)],
            }],
            vec![HirStmt::Match {
                subject: HirExpr::IntLiteral(0),
                cases: vec![HirMatchCase {
                    pattern: HirPattern::Wildcard,
                    guard: None,
                    body: vec![HirStmt::Return(None)],
                }],
            }],
            vec![store("b"), leaf_without_return()],
            vec![],
        ];
        for (index, body) in clean.iter().enumerate() {
            assert!(
                !body_returns_inside_finally(body),
                "shape {index}: {body:?}"
            );
        }
    }

    /// The transitivity pin, and the exact shape of the host crash that
    /// prompted this predicate: `return` inside a `try`/`finally` inside a
    /// `while` inside a `finally` body.
    ///
    /// The first assertion is what makes the second one mean something: the
    /// `finally` clause's own direct children contain no `Return` at all, so
    /// a walk of those children alone answers `false` and re-admits the
    /// segfaulting program. Only the transitive walk sees it.
    #[test]
    fn the_reproductions_return_is_four_blocks_below_its_finally() {
        let inner = try_stmt(
            vec![HirStmt::Return(Some(HirExpr::Name("b".to_string())))],
            vec![],
            vec![],
            vec![HirStmt::Raise {
                exc: None,
                cause: None,
            }],
        );
        let finalbody = vec![
            store("a"),
            HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: vec![inner],
            },
        ];
        assert!(
            !finalbody
                .iter()
                .any(|stmt| matches!(stmt, HirStmt::Return(_))),
            "the finally clause's own children must hold no `Return`, or this \
             test would pass for a non-transitive walk too"
        );
        assert!(body_returns_inside_finally(&[try_stmt(
            vec![HirStmt::Return(Some(HirExpr::Name("a".to_string())))],
            vec![],
            vec![],
            finalbody,
        )]));
    }

    /// A statement carrying no nested block and no `return` of its own, so
    /// the finally-side leaf arm answers it.
    fn leaf_without_return() -> HirStmt {
        HirStmt::ExprStmt(HirExpr::IntLiteral(0))
    }

    fn try_stmt(
        body: Vec<HirStmt>,
        handlers: Vec<HirExceptHandler>,
        orelse: Vec<HirStmt>,
        finalbody: Vec<HirStmt>,
    ) -> HirStmt {
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
    }

    // -----------------------------------------------------------------
    // Part 2 of #1175 (#1179): `body_returns_slice_of`.
    // -----------------------------------------------------------------

    /// `return <name>[1:]`, the only statement shape that can answer
    /// `true`.
    fn slice_return(name: &str) -> HirStmt {
        HirStmt::Return(Some(HirExpr::Slice {
            base: Box::new(HirExpr::Name(name.to_string())),
            start: Some(Box::new(HirExpr::IntLiteral(1))),
            stop: None,
            step: None,
        }))
    }

    /// Every shape that must answer `false`, at the top level where the
    /// leaf arm sees it directly. Answering `true` for any of these would
    /// widen the compiled function's signature by three out-pointers the
    /// `Return` site never writes, leaving them indeterminate.
    #[test]
    fn only_a_slice_return_of_the_named_buffer_is_reported() {
        assert!(body_returns_slice_of(&[slice_return("b")], "b"));
        assert!(!body_returns_slice_of(&[slice_return("other")], "b"));
        assert!(!body_returns_slice_of(&[], "b"));
        let no: Vec<HirStmt> = vec![
            leaf(),
            store("b"),
            // A bare `return b` is Part 1's whole-view egress, which
            // carries no bounds.
            HirStmt::Return(Some(HirExpr::Name("b".to_string()))),
            // A `step` is refused by `pycc_types`, and the two walks must
            // agree that it is not this shape.
            HirStmt::Return(Some(HirExpr::Slice {
                base: Box::new(HirExpr::Name("b".to_string())),
                start: None,
                stop: None,
                step: Some(Box::new(HirExpr::IntLiteral(2))),
            })),
            // A slice whose base is not a bare name.
            HirStmt::Return(Some(HirExpr::Slice {
                base: Box::new(HirExpr::IntLiteral(0)),
                start: None,
                stop: None,
                step: None,
            })),
            // A slice that is not the whole operand of the `return`.
            HirStmt::Return(Some(HirExpr::Subscript {
                base: Box::new(HirExpr::Slice {
                    base: Box::new(HirExpr::Name("b".to_string())),
                    start: None,
                    stop: None,
                    step: None,
                }),
                index: Box::new(HirExpr::IntLiteral(0)),
            })),
        ];
        for stmt in no {
            assert!(
                !body_returns_slice_of(std::slice::from_ref(&stmt), "b"),
                "{stmt:?}"
            );
        }
    }

    /// Every block-carrying variant, each with the slice `return` in one
    /// nested position. This walk is one of the two independent
    /// computations of the out-slot fact, so a variant it fails to recurse
    /// into is an ABI mismatch between the generated wrapper's call and the
    /// compiled function's real arity.
    #[test]
    fn a_slice_return_is_found_inside_every_nested_block() {
        let nested: Vec<Vec<HirStmt>> = vec![
            vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![slice_return("b")],
                orelse: vec![],
            }],
            vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![],
                orelse: vec![slice_return("b")],
            }],
            vec![HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: vec![slice_return("b")],
            }],
            vec![HirStmt::ForRange {
                var: "i".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(1),
                step: HirExpr::IntLiteral(1),
                body: vec![slice_return("b")],
            }],
            vec![HirStmt::ForList {
                var: "i".to_string(),
                list: "xs".to_string(),
                body: vec![slice_return("b")],
            }],
            vec![HirStmt::ForObject {
                var: "i".to_string(),
                iter: Box::new(HirExpr::IntLiteral(0)),
                body: vec![slice_return("b")],
            }],
            vec![HirStmt::Match {
                subject: HirExpr::IntLiteral(0),
                cases: vec![HirMatchCase {
                    pattern: HirPattern::Wildcard,
                    guard: None,
                    body: vec![slice_return("b")],
                }],
            }],
            vec![try_stmt(vec![slice_return("b")], vec![], vec![], vec![])],
            vec![try_stmt(
                vec![],
                vec![HirExceptHandler {
                    exc_type: None,
                    name: None,
                    body: vec![slice_return("b")],
                }],
                vec![],
                vec![],
            )],
            vec![try_stmt(vec![], vec![], vec![slice_return("b")], vec![])],
            vec![try_stmt(vec![], vec![], vec![], vec![slice_return("b")])],
            vec![HirStmt::TryStar {
                body: vec![slice_return("b")],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![],
            }],
            vec![HirStmt::TryStar {
                body: vec![],
                handlers: vec![HirExceptHandler {
                    exc_type: None,
                    name: None,
                    body: vec![slice_return("b")],
                }],
                orelse: vec![],
                finalbody: vec![],
            }],
            vec![HirStmt::TryStar {
                body: vec![],
                handlers: vec![],
                orelse: vec![slice_return("b")],
                finalbody: vec![],
            }],
            vec![HirStmt::TryStar {
                body: vec![],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![slice_return("b")],
            }],
            // Two levels deep, so the recursion is not merely one-deep.
            vec![HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::If {
                    test: HirExpr::BoolLiteral(true),
                    body: vec![slice_return("b")],
                    orelse: vec![],
                }],
            }],
        ];
        for (index, body) in nested.iter().enumerate() {
            assert!(body_returns_slice_of(body, "b"), "shape {index}: {body:?}");
            assert!(
                !body_returns_slice_of(body, "other"),
                "shape {index}: {body:?}"
            );
        }
    }
}
