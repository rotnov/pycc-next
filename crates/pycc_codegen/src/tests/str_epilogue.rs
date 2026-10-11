//! #1054: str parameter and local release in function epilogues.

use super::*;

/// How many `str` slots the emitted epilogues release, module-wide. Counted
/// through the loaded operand's own name, because LLVM prints no result name
/// for a `void` call -- the `"str_epilogue_decref"` this crate passes
/// `build_call` never appears in the textual IR.
fn str_epilogue_releases(ir: &str) -> usize {
    ir.matches("@pycc_rt_str_decref(ptr %str_epilogue_live")
        .count()
}

/// How many functions got a `str` epilogue at all. Counts the block label's
/// own definition site, not the branches into it.
fn str_epilogue_blocks(ir: &str) -> usize {
    ir.matches("pycc_str_epilogue:").count()
}

/// (a) The plain case: a `str` parameter, an ordinary `return`. The return
/// routes through the synthetic frame (no edit to `MirStmt::Return` was
/// needed), the epilogue releases the parameter slot, and the function's
/// single `ret` reads the epilogue's own `ret_slot`.
#[test]
fn issue_1054_a_str_parameter_is_released_on_a_normal_return() {
    compile_items_checking_ir(
        "issue_1054_a_normal_return",
        vec![MirItem::Function {
            name: "echo".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::Str,
            body: vec![MirStmt::Return(Some(str_name("s")))],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 1, "{ir}");
            assert!(ir.contains("str_epilogue_ret_slot"), "{ir}");
            // The returned duplicate is increfd by the `return` itself, so
            // the epilogue's release balances the caller's incoming
            // reference rather than the value being handed back.
            assert!(ir.contains("pycc_rt_str_incref"), "{ir}");
        },
    );
}

/// (b) A `None`-returning function that falls off the end of its body: the
/// implicit `return None` never passes through `finally_stack`, so it is one
/// of the two sites that needed an explicit branch into the epilogue.
#[test]
fn issue_1054_b_a_none_returning_fall_through_reaches_the_epilogue() {
    compile_items_checking_ir(
        "issue_1054_b_fall_through",
        vec![MirItem::Function {
            name: "consume".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::None,
            body: vec![MirStmt::NoOp],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 1, "{ir}");
            // A `None` ABI gets no `ret_slot`: the epilogue ends in
            // `ret void`.
            assert!(!ir.contains("str_epilogue_ret_slot"), "{ir}");
        },
    );
}

/// (c) A `return` from inside a `try`/`finally`. `emit_try`'s own `ret_bb`
/// finds the synthetic frame as its enclosing `finally_stack` entry and
/// propagates outward to it, so this path reaches the epilogue with no edit
/// to `exception.rs` at all.
#[test]
fn issue_1054_c_a_return_through_a_finally_reaches_the_epilogue() {
    compile_items_checking_ir(
        "issue_1054_c_try_finally",
        vec![MirItem::Function {
            name: "guarded".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::Str,
            body: vec![MirStmt::Try {
                body: vec![MirStmt::Return(Some(str_name("s")))],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![MirStmt::NoOp],
            }],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 1, "{ir}");
            assert!(ir.contains("try_ret_slot"), "{ir}");
        },
    );
}

/// (d) The same for `try*` (PEP 654), whose `ret_bb` is a separate emitter
/// with its own copy of the propagate-to-the-enclosing-frame logic.
#[test]
fn issue_1054_d_a_return_through_a_try_star_finally_reaches_the_epilogue() {
    compile_items_checking_ir(
        "issue_1054_d_try_star_finally",
        vec![MirItem::Function {
            name: "guarded_star".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::Str,
            body: vec![MirStmt::TryStar {
                body: vec![MirStmt::Return(Some(str_name("s")))],
                // `emit_try_star` requires at least one clause -- an
                // `except*` with no handler is not a shape the lowerer can
                // produce, so the fixture carries a terminating one.
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![MirStmt::Return(Some(MirExpr::StringLiteral(
                        "handled".to_string(),
                    )))],
                }],
                orelse: vec![],
                finalbody: vec![MirStmt::NoOp],
            }],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 1, "{ir}");
            assert!(ir.contains("trystar_ret_slot"), "{ir}");
        },
    );
}

/// (e) The exceptional exit. An in-flight exception does not make the
/// references this function owns somebody else's problem: `exception_exit`
/// stores its neutral carrier into the epilogue's `ret_slot` and branches in
/// rather than returning directly.
#[test]
fn issue_1054_e_the_exception_exit_reaches_the_epilogue() {
    compile_items_checking_ir(
        "issue_1054_e_exception_exit",
        vec![MirItem::Function {
            name: "always_raises".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::Str,
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1,
                    class_name: "ValueError".to_string(),
                    message: MirExpr::StringLiteral("boom".to_string()),
                },
                frame_function: "always_raises".to_string(),
            }],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 1, "{ir}");
            assert!(ir.contains("exception_exit"), "{ir}");
            // The `unreachable` fall-through arm is untouched: a body that
            // always raises still terminates statically.
            assert!(ir.contains("unreachable"), "{ir}");
        },
    );
}

/// (f) A `str` local that a given path never assigned still holds the null
/// `storage_slot_at_entry` initialized it with, and `pycc_rt_str_decref` is
/// a documented no-op on null -- so the epilogue needs no per-slot guard.
#[test]
fn issue_1054_f_an_unassigned_str_local_is_released_unconditionally() {
    compile_items_checking_ir(
        "issue_1054_f_unassigned_local",
        vec![MirItem::Function {
            name: "maybe_binds".to_string(),
            params: vec![("flag".to_string(), Ty::Bool)],
            return_ty: Ty::None,
            body: vec![MirStmt::If {
                test: MirExpr::Name {
                    name: "flag".to_string(),
                    ty: Ty::Bool,
                },
                body: vec![MirStmt::Assign {
                    target: "t".to_string(),
                    value: MirExpr::StringLiteral("bound".to_string()),
                }],
                orelse: vec![],
            }],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 1, "{ir}");
        },
    );
}

/// (g) The gate itself: a function with no `str` (or other owned) slot pushes no synthetic
/// frame and emits no epilogue, which is what keeps the "no outer
/// `finally_stack` entry" arms of `MirStmt::Return` and of `emit_try`/
/// `emit_try_star`'s `ret_bb` reachable for the tests that cover them.
#[test]
fn issue_1054_g_a_str_free_function_gets_no_epilogue() {
    compile_items_checking_ir(
        "issue_1054_g_no_str_slot",
        vec![MirItem::Function {
            name: "arithmetic".to_string(),
            params: vec![("n".to_string(), Ty::Float)],
            return_ty: Ty::Float,
            body: vec![MirStmt::Return(Some(MirExpr::Name {
                name: "n".to_string(),
                ty: Ty::Float,
            }))],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 0, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 0, "{ir}");
        },
    );
}

/// (h) The type-confusion hazard the entry-slot *snapshot* exists to
/// prevent. `except ValueError as s` rebinds the name `s` mid-function to a
/// slot holding an exception instance, not a `PyStrObj`. A walk of the
/// live locals at exit would decref that instance pointer as a string; the
/// snapshot released here is still the parameter's own entry-block slot, and
/// there is exactly one release, not two.
#[test]
fn issue_1054_h_an_except_binding_shadowing_a_str_parameter_is_not_released() {
    compile_items_checking_ir(
        "issue_1054_h_except_shadow",
        vec![MirItem::Function {
            name: "shadowed".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::None,
            body: vec![MirStmt::Try {
                body: vec![MirStmt::Raise {
                    exception: MirExceptionValue::Constructed {
                        type_tag: 1,
                        class_name: "ValueError".to_string(),
                        message: MirExpr::StringLiteral("boom".to_string()),
                    },
                    frame_function: "shadowed".to_string(),
                }],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: Some("s".to_string()),
                    binding_ty: Some(Ty::Instance(Box::new("ValueError".to_string()))),
                    body: vec![MirStmt::NoOp],
                }],
                orelse: vec![],
                finalbody: vec![],
            }],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(
                str_epilogue_releases(ir),
                1,
                "only the parameter's own entry-block slot is in the snapshot: {ir}"
            );
        },
    );
}

/// (i) The compiled-to-compiled call boundary, duplicate-argument shape. The
/// caller reads a bare `Name`, so `incref_if_str_duplicate` increfs before
/// the call and the callee's slot owns a reference of its own -- newly
/// load-bearing, because before this change the callee simply leaked it.
#[test]
fn issue_1054_i_a_bare_name_argument_is_increfd_by_the_caller_and_released_by_the_callee() {
    compile_items_checking_ir(
        "issue_1054_i_duplicate_argument",
        vec![
            MirItem::Function {
                name: "callee".to_string(),
                params: vec![("s".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![MirStmt::NoOp],
            },
            MirItem::Function {
                name: "caller".to_string(),
                params: vec![("s".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "callee".to_string(),
                    args: vec![str_name("s")],
                    ty: Ty::None,
                })],
            },
        ],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 2, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 2, "{ir}");
            assert!(
                ir.contains("pycc_rt_str_incref"),
                "the caller must incref a bare-`Name` argument it keeps: {ir}"
            );
        },
    );
}

/// (j) The same boundary, fresh-temporary shape. `a + b` is not a bare
/// `Name`, so no incref happens: the concatenation's own refcount-1 object
/// transfers into the callee's slot, which is then the only owner and the
/// only releaser.
#[test]
fn issue_1054_j_a_fresh_temporary_argument_is_owned_solely_by_the_callee() {
    compile_items_checking_ir(
        "issue_1054_j_fresh_argument",
        vec![
            MirItem::Function {
                name: "callee".to_string(),
                params: vec![("s".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![MirStmt::NoOp],
            },
            MirItem::Function {
                name: "caller".to_string(),
                params: vec![("a".to_string(), Ty::Str), ("b".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "callee".to_string(),
                    args: vec![MirExpr::BinOp {
                        op: BinOpKind::Add,
                        left: Box::new(str_name("a")),
                        right: Box::new(str_name("b")),
                        ty: Ty::Str,
                    }],
                    ty: Ty::None,
                })],
            },
        ],
        |ir| {
            // Three slots: the callee's `s`, and the caller's `a` and `b`.
            assert_eq!(str_epilogue_blocks(ir), 2, "{ir}");
            assert_eq!(str_epilogue_releases(ir), 3, "{ir}");
            assert!(
                ir.contains("pycc_rt_str_concat"),
                "the argument must be a freshly built temporary: {ir}"
            );
        },
    );
}

/// (k) The `except*` mirror of (h). `emit_try_star` applies the same
/// mid-function `locals.insert(binding_name, ...)` overwrite that
/// `emit_try` does, so `except* ValueError as s` over a `str` parameter is
/// the same type-confusion hazard through a second, independently written
/// lowering path. The entry-block snapshot is what makes both safe, and this
/// test pins the `TryStar` arm of it rather than inferring it from (h).
#[test]
fn issue_1054_k_an_except_star_binding_shadowing_a_str_parameter_is_not_released() {
    compile_items_checking_ir(
        "issue_1054_k_except_star_shadow",
        vec![MirItem::Function {
            name: "shadowed_star".to_string(),
            params: vec![("s".to_string(), Ty::Str)],
            return_ty: Ty::None,
            body: vec![MirStmt::TryStar {
                body: vec![MirStmt::Raise {
                    exception: MirExceptionValue::Constructed {
                        type_tag: 1,
                        class_name: "ValueError".to_string(),
                        message: MirExpr::StringLiteral("boom".to_string()),
                    },
                    frame_function: "shadowed_star".to_string(),
                }],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: Some("s".to_string()),
                    binding_ty: Some(Ty::Instance(Box::new("ValueError".to_string()))),
                    body: vec![MirStmt::NoOp],
                }],
                orelse: vec![],
                finalbody: vec![],
            }],
        }],
        |ir| {
            assert_eq!(str_epilogue_blocks(ir), 1, "{ir}");
            assert_eq!(
                str_epilogue_releases(ir),
                1,
                "only the parameter's own entry-block slot is in the snapshot: {ir}"
            );
        },
    );
}
