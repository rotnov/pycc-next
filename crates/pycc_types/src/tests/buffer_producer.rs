//! The artifact-owned buffer producer: what `crate::buffer` admits, and
//! every refusal that bounds it (Part 2a of #1142, issue #1165).
//!
//! Its own file rather than more lines in `tests.rs`, under AGENTS.md's
//! decomposability rule and the layout `constraints.rs` beside it
//! established. As a child module it sees the parent's private items
//! through `use super::*`, so nothing needed widened visibility.
//!
//! Every test here is non-`#[ignore]`d on purpose: CI's `llvm-cov` job runs
//! without `--include-ignored`, so an ignored test contributes nothing to
//! the changed-line denominator D-242 pins at 100%.

use super::*;

/// `def f(<params>) -> <ret>: <body>`, with no class table.
fn func(params: Vec<(String, Ty)>, return_ty: Ty, body: Vec<HirStmt>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params,
            return_ty,
            body,
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

/// `<callee>(<args>)`.
fn call(callee: &str, args: Vec<HirExpr>) -> HirExpr {
    HirExpr::Call {
        callee: callee.to_string(),
        args,
    }
}

/// `a = <callee>(4)`.
fn alloc_four(callee: &str) -> HirStmt {
    HirStmt::Assign {
        target: "a".to_string(),
        value: call(callee, vec![HirExpr::IntLiteral(4)]),
    }
}

/// `a[0]`.
fn element() -> HirExpr {
    HirExpr::Subscript {
        base: Box::new(HirExpr::Name("a".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    }
}

/// A module of two functions: an unannotated private helper `_h(n)` whose
/// body is `body`, and an annotated `f()` that calls it with `4`.
///
/// `Ty::Infer` on the helper is what routes the module through the
/// *constraint solver* rather than the concrete fast path, which is the only
/// way to reach `constraints.rs`'s own admitting seam for the producer. The
/// check phase's seam is reached by every other test in this file.
fn unannotated_helper_module(body: Vec<HirStmt>, f_return: Ty) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("n".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body,
            },
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: f_return,
                body: vec![HirStmt::Return(Some(call(
                    "_h",
                    vec![HirExpr::IntLiteral(4)],
                )))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

/// `b = a` -- a bare read of an owned name, which is what makes a masked
/// producer refusal *observable*: it is the use that raises the owned-buffer
/// `C0001` the solver's own answer would otherwise displace the correct
/// diagnostic with. A subscript read would not, since `a[0]` is admitted.
fn read_whole_buffer() -> HirStmt {
    HirStmt::Assign {
        target: "b".to_string(),
        value: HirExpr::Name("a".to_string()),
    }
}

/// The text of `buffer::owned_buffer_use_unsupported`, for the "and the
/// owned-buffer refusal is *absent*" half of every assertion below. Pinning
/// only the expected code passes just as well when both diagnostics are
/// produced and the wrong one wins, which is exactly the state these tests
/// exist to refuse.
const OWNED_BUFFER_REFUSAL: &str =
    "bound to buffer storage this `pycc build --ext` artifact allocated";

/// The text of `buffer::buffer_parameter_rebinding`, for the second half of
/// the two-directional assertion below. Clearing provenance while leaving
/// the stale `Ty::MemoryView` binding in place would swap the owned refusal
/// for this one, so a test that pinned only [`OWNED_BUFFER_REFUSAL`]'s
/// absence would pass on that non-fix.
const PARAMETER_BUFFER_REFUSAL: &str = "a buffer parameter of a `pycc build --ext` export";

/// The fragment of `crate::buffer::buffer_return_inside_finally` these
/// assertions pin. Kept distinct from [`OWNED_BUFFER_REFUSAL`] on purpose:
/// the two refusals answer different questions, and a test that accepted
/// either would pass for a fall-through that never reached the new one.
const RETURN_IN_FINALLY_REFUSAL: &str = "a `return` inside a `finally` clause";

/// The tail of `crate::buffer::owned_buffer_use_unsupported`, for the "and
/// the *other* owned-buffer refusal is absent" half of the assertions below.
/// [`OWNED_BUFFER_REFUSAL`] cannot serve there: the new message deliberately
/// opens with the same "bound to buffer storage this artifact allocated"
/// clause -- it is about the same kind of name -- so only the tail
/// distinguishes a refusal that reached review round 5's conjunct from one
/// that fell through to the #1165 message.
const OWNED_BUFFER_REFUSAL_TAIL: &str = "#1165 admits such a buffer only inside the function";

/// `try: <body>  finally: while True: <finalbody>`, the shape the
/// pre-existing PEP 765 `L0001` rule deliberately does not reach: it follows
/// CPython in clearing its `finally` context on loop entry, so a `return`
/// under the loop is valid Python that reaches the buffer egress.
fn try_returning_from_a_loop_in_finally(body: Vec<HirStmt>, returned: HirExpr) -> HirStmt {
    HirStmt::Try {
        body,
        handlers: Vec::new(),
        orelse: Vec::new(),
        finalbody: vec![HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Return(Some(returned))],
        }],
    }
}

// Topic submodules split out of this file.
mod producers;
mod solver_and_final;
mod rebinding_and_egress;
mod unbound_and_finally;
mod caller_owned_and_slices;
