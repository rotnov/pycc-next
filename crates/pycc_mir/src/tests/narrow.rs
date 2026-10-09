//! Issue #769 (Part 2 of #747): flow-sensitive `Optional[T]` narrowing at
//! the MIR layer -- `stmt::lower_stmt`'s `HirStmt::If` arm pushing/popping
//! the `$narrowed:{name}` scope sentinel (`push_narrowing`/`kill_narrowing`
//! in the crate root), and `expr::lower_expr`'s `HirExpr::Name` arm
//! consulting it (`narrowed_ty`) to emit `MirExpr::OptionalUnwrap`.
//!
//! Mirrors `pycc_types::narrow`'s own test suite one layer down: this crate
//! trusts that `pycc_types::check` has already proven every narrowing this
//! module exercises sound, so these tests only prove the *lowering*, not
//! the checker's own eligibility rules (compound tests, non-`Optional`
//! names, etc. -- those never reach `pycc_mir::build` at all, since
//! `pycc_types::check` runs first and would already have rejected an
//! unsound program).

use crate::*;
use pycc_hir::{CmpOpKind, HirExpr, HirItem, HirModule, HirStmt, Ty};

/// `x: int | None = 5` at module scope, ahead of every test below --
/// shared setup for "`x` is already known `Optional[int]` before the `if`".
fn optional_int_decl(target: &str) -> HirStmt {
    HirStmt::AnnAssign {
        is_final: false,
        target: target.to_string(),
        annotation: Ty::Optional(Box::new(Ty::Int)),
        value: Some(HirExpr::IntLiteral(5)),
    }
}

fn is_not_none(name: &str) -> HirExpr {
    HirExpr::Compare {
        op: CmpOpKind::IsNot,
        left: Box::new(HirExpr::Name(name.to_string())),
        right: Box::new(HirExpr::NoneLiteral),
    }
}

fn is_none(name: &str) -> HirExpr {
    HirExpr::Compare {
        op: CmpOpKind::Is,
        left: Box::new(HirExpr::Name(name.to_string())),
        right: Box::new(HirExpr::NoneLiteral),
    }
}

fn print_x(name: &str) -> HirStmt {
    HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Name(name.to_string())],
    })
}

fn module(items: Vec<HirItem>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

/// `x: int | None = 5; if x is not None: print(x)` -- the body's read of
/// `x` is narrowed, so it must lower to `MirExpr::OptionalUnwrap` wrapping
/// the still-`Optional`-typed `MirExpr::Name`, not a bare `Name { ty:
/// Optional(Int) }`.
#[test]
fn is_not_none_narrows_the_body_read_to_an_optional_unwrap() {
    let hir = module(vec![
        HirItem::TopLevelStmt(optional_int_decl("x")),
        HirItem::TopLevelStmt(HirStmt::If {
            test: is_not_none("x"),
            body: vec![print_x("x")],
            orelse: vec![],
        }),
    ]);
    let mir = build(&hir);
    let MirItem::TopLevelStmt(MirStmt::If { body, orelse, .. }) = &mir.items[1] else {
        panic!("expected the second item to be the lowered `if`");
    };
    assert!(orelse.is_empty());
    assert_eq!(
        body,
        &vec![MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::OptionalUnwrap(
                Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Optional(Box::new(Ty::Int)),
                }),
                Box::new(Ty::Int),
            )],
            ty: Ty::None,
        })]
    );
}

/// `x: int | None = 5; if x is None: pass else: print(x)` -- the mirror
/// polarity: only `orelse`'s read is narrowed.
#[test]
fn is_none_narrows_the_orelse_read_to_an_optional_unwrap() {
    let hir = module(vec![
        HirItem::TopLevelStmt(optional_int_decl("x")),
        HirItem::TopLevelStmt(HirStmt::If {
            test: is_none("x"),
            body: vec![],
            orelse: vec![print_x("x")],
        }),
    ]);
    let mir = build(&hir);
    let MirItem::TopLevelStmt(MirStmt::If { body, orelse, .. }) = &mir.items[1] else {
        panic!("expected the second item to be the lowered `if`");
    };
    assert!(body.is_empty());
    assert_eq!(
        orelse,
        &vec![MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::OptionalUnwrap(
                Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Optional(Box::new(Ty::Int)),
                }),
                Box::new(Ty::Int),
            )],
            ty: Ty::None,
        })]
    );
}

/// The non-narrowed branch of an `is not None` test keeps reading `x` as
/// plain `Optional(Int)` -- no `OptionalUnwrap` on the `orelse` side.
#[test]
fn is_not_none_does_not_narrow_the_orelse_read() {
    let hir = module(vec![
        HirItem::TopLevelStmt(optional_int_decl("x")),
        HirItem::TopLevelStmt(HirStmt::If {
            test: is_not_none("x"),
            body: vec![],
            orelse: vec![print_x("x")],
        }),
    ]);
    let mir = build(&hir);
    let MirItem::TopLevelStmt(MirStmt::If { orelse, .. }) = &mir.items[1] else {
        panic!("expected the second item to be the lowered `if`");
    };
    assert_eq!(
        orelse,
        &vec![MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        })]
    );
}

/// Narrowing never leaks past the `if`: a read of `x` after the statement
/// (a sibling in the same top-level sequence) must still see plain
/// `Optional(Int)`, proving the `$narrowed:{name}` sentinel this arm pushes
/// is popped again once the narrowed branch finishes lowering.
#[test]
fn narrowing_does_not_leak_past_the_if() {
    let hir = module(vec![
        HirItem::TopLevelStmt(optional_int_decl("x")),
        HirItem::TopLevelStmt(HirStmt::If {
            test: is_not_none("x"),
            body: vec![print_x("x")],
            orelse: vec![],
        }),
        HirItem::TopLevelStmt(print_x("x")),
    ]);
    let mir = build(&hir);
    assert_eq!(
        mir.items[2],
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        }))
    );
}

/// A reassignment inside the narrowed body kills the narrowing for every
/// read after it, within the same branch: `if x is not None: x = 5;
/// print(x)` -- the `print(x)` here must read plain `Optional(Int)`, not
/// `OptionalUnwrap`, mirroring `pycc_types::narrow`'s own kill-on-assign
/// rule (`crate::kill_narrowing`, called from `lower_stmt`'s `Assign` arm).
#[test]
fn a_reassignment_inside_the_narrowed_body_kills_the_narrowing() {
    let hir = module(vec![
        HirItem::TopLevelStmt(optional_int_decl("x")),
        HirItem::TopLevelStmt(HirStmt::If {
            test: is_not_none("x"),
            body: vec![
                HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(5),
                },
                print_x("x"),
            ],
            orelse: vec![],
        }),
    ]);
    let mir = build(&hir);
    let MirItem::TopLevelStmt(MirStmt::If { body, .. }) = &mir.items[1] else {
        panic!("expected the second item to be the lowered `if`");
    };
    assert_eq!(
        body[1],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        })
    );
}

/// D-068 review of #780/#774's interaction (blocker finding 1): a walrus
/// target inside a narrowed `if` branch is a reassignment exactly like a
/// plain `x = None` (the test directly above this one) and must kill `x`'s
/// narrowing sentinel the same way. Before this fix,
/// `expr::pre_bind_named_expr_targets`'s `HirExpr::NamedExpr` arm called
/// `bind_variable` but never `kill_narrowing`, so the read right after
/// `(x := 5)` kept unconditionally lowering to `MirExpr::OptionalUnwrap`
/// wrapping the *original* `Optional[int]`-typed name, exactly as if the
/// walrus reassignment had never happened.
#[test]
fn a_walrus_reassignment_inside_the_narrowed_body_kills_the_narrowing() {
    let hir = module(vec![
        HirItem::TopLevelStmt(optional_int_decl("x")),
        HirItem::TopLevelStmt(HirStmt::If {
            test: is_not_none("x"),
            body: vec![
                HirStmt::ExprStmt(HirExpr::NamedExpr {
                    name: "x".to_string(),
                    value: Box::new(HirExpr::IntLiteral(5)),
                }),
                print_x("x"),
            ],
            orelse: vec![],
        }),
    ]);
    let mir = build(&hir);
    let MirItem::TopLevelStmt(MirStmt::If { body, .. }) = &mir.items[1] else {
        panic!("expected the second item to be the lowered `if`");
    };
    assert_eq!(
        body[1],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        })
    );
}

/// A test naming a variable that is not currently `Optional`-scoped is
/// never treated as narrowing-eligible -- `if flag is not None:` where
/// `flag` is a plain `bool` never reaches `pycc_mir::build` in practice
/// (`pycc_types::check`'s own T0021 rejects an `is`/`is not None` operand
/// that is not `Optional` before HIR lowering ever gets here), but this
/// lowering pass's own `narrows_body`/`narrows_orelse` computation must
/// still fail closed (`None`) rather than panicking, matching
/// `pycc_types::narrow::narrowing_target`'s identical fail-closed
/// `_ => None` arm for the same shape.
#[test]
fn a_plain_bool_test_reads_normally_with_no_unwrap() {
    let hir = module(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "flag".to_string(),
            value: HirExpr::BoolLiteral(true),
        }),
        HirItem::TopLevelStmt(HirStmt::If {
            test: HirExpr::Name("flag".to_string()),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("flag".to_string())],
            })],
            orelse: vec![],
        }),
    ]);
    let mir = build(&hir);
    let MirItem::TopLevelStmt(MirStmt::If { body, .. }) = &mir.items[1] else {
        panic!("expected the second item to be the lowered `if`");
    };
    assert_eq!(
        body,
        &vec![MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "flag".to_string(),
                ty: Ty::Bool,
            }],
            ty: Ty::None,
        })]
    );
}

/// The early-return continuation shape (issue #769's second narrowing
/// mechanism, alongside in-branch narrowing above): `def f(x: int | None):
/// if x is None: return; print(x)` -- the read of `x` in the statement
/// *after* the `if` (a sibling in the same function-body sequence, not a
/// branch of the `if` itself) must still be narrowed, since reaching that
/// point is only possible via the implicit "`x` is not `None`" path. Proves
/// `lower_stmt_sequence`/`apply_post_if_narrowing` (wired into
/// `lower_item`'s `HirItem::Function` arm) actually fires, mirroring
/// `pycc_types::narrow::apply_post_if_narrowing`'s identical checker-layer
/// behavior -- this is the exact shape `tests/fixtures/pep_0604_union.py`'s
/// `describe` function exercises end-to-end.
#[test]
fn an_early_return_guard_narrows_the_read_after_it_in_a_function_body() {
    let hir = module(vec![HirItem::Function {
        name: "describe".to_string(),
        params: vec![("x".to_string(), Ty::Optional(Box::new(Ty::Int)))],
        return_ty: Ty::None,
        body: vec![
            HirStmt::If {
                test: is_none("x"),
                body: vec![HirStmt::Return(None)],
                orelse: vec![],
            },
            print_x("x"),
        ],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    assert_eq!(
        body[1],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::OptionalUnwrap(
                Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Optional(Box::new(Ty::Int)),
                }),
                Box::new(Ty::Int),
            )],
            ty: Ty::None,
        })
    );
}

/// A narrowing fact established by an early-return guard *nested inside* an
/// unrelated `if`'s body must not leak past that `if`'s own close: `def
/// f(cond, x): if cond: (if x is None: return); print(x)` (after the outer
/// `if`) -- the outer `if`'s own test says nothing about `x`, so `x` must
/// still read as plain `Optional(Int)` once the outer `if` finishes, even
/// though the read *inside* the outer `if`'s body (right after the nested
/// early-return guard) is correctly narrowed. This is the isolation fix:
/// without `lower_scoped_body`'s snapshot/restore around every *nested*
/// body (as opposed to `lower_stmt_sequence`'s own top-level, non-isolated
/// use), the nested `apply_post_if_narrowing` hit would otherwise leak its
/// `$narrowed:x` sentinel into `scopes`' single function-level frame and
/// incorrectly narrow the read after the outer `if` too.
#[test]
fn a_nested_early_return_narrowing_does_not_leak_past_the_enclosing_if() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("cond".to_string(), Ty::Bool),
            ("x".to_string(), Ty::Optional(Box::new(Ty::Int))),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::If {
                test: HirExpr::Name("cond".to_string()),
                body: vec![
                    HirStmt::If {
                        test: is_none("x"),
                        body: vec![HirStmt::Return(None)],
                        orelse: vec![],
                    },
                    print_x("x"),
                ],
                orelse: vec![],
            },
            print_x("x"),
        ],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the first function statement to be the lowered outer `if`");
    };
    // Inside the outer `if`'s own body, right after the nested guard: still
    // narrowed (proves the nested guard's own narrowing did take effect for
    // its own enclosing sequence).
    assert_eq!(
        outer_body[1],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::OptionalUnwrap(
                Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Optional(Box::new(Ty::Int)),
                }),
                Box::new(Ty::Int),
            )],
            ty: Ty::None,
        })
    );
    // After the outer `if` entirely: back to plain `Optional(Int)`, not
    // leaked.
    assert_eq!(
        body[1],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        })
    );
}

/// `is_none`/`is_not_none` on a name that is *not* currently `Optional`
/// (e.g. a plain `bool`) never narrows either branch in the in-branch
/// (`HirStmt::If`'s own `narrows_body`/`narrows_orelse`) mechanism -- the
/// `_ => None` fail-closed arm of `stmt::lower_stmt`'s own `narrowing`
/// computation. This never happens via a real, type-checked program (the
/// checker's own `T0021` already rejects `is`/`is not None` against a
/// non-`Optional` operand before HIR lowering), but this hand-built-HIR MIR
/// test bypasses the checker entirely, exactly like
/// `a_plain_bool_test_reads_normally_with_no_unwrap` above does for a
/// non-none-test shape -- this one instead pins the fail-closed behavior
/// for a *recognized* `is`/`is not None` shape whose operand just isn't
/// `Optional`-scoped.
#[test]
fn an_is_none_test_on_a_non_optional_name_narrows_neither_branch() {
    let hir = module(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "flag".to_string(),
            value: HirExpr::BoolLiteral(true),
        }),
        HirItem::TopLevelStmt(HirStmt::If {
            test: is_none("flag"),
            body: vec![],
            orelse: vec![print_x("flag")],
        }),
    ]);
    let mir = build(&hir);
    let MirItem::TopLevelStmt(MirStmt::If { orelse, .. }) = &mir.items[1] else {
        panic!("expected the second item to be the lowered `if`");
    };
    assert_eq!(
        orelse,
        &vec![MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "flag".to_string(),
                ty: Ty::Bool,
            }],
            ty: Ty::None,
        })]
    );
}

/// The early-return-continuation mechanism (`apply_post_if_narrowing`) has
/// its own separate `if let Ty::Optional(inner) = lookup(scopes, name) { .. }`
/// fail-closed guard, distinct from `stmt::lower_stmt`'s in-branch one
/// exercised just above: `if flag is None: return` on a non-`Optional`
/// `flag` recognizes `definitely_terminates` and the none-test shape, but
/// must still not push a `$narrowed:flag` sentinel, since `flag` is not
/// `Optional`-scoped. Never reachable via a real type-checked program for
/// the same reason as the test above; this pins `apply_post_if_narrowing`'s
/// own identical fail-closed behavior directly.
#[test]
fn an_early_return_guard_on_a_non_optional_name_does_not_narrow_the_continuation() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![("flag".to_string(), Ty::Bool)],
        return_ty: Ty::None,
        body: vec![
            HirStmt::If {
                test: is_none("flag"),
                body: vec![HirStmt::Return(None)],
                orelse: vec![],
            },
            print_x("flag"),
        ],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    assert_eq!(
        body[1],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "flag".to_string(),
                ty: Ty::Bool,
            }],
            ty: Ty::None,
        })
    );
}

/// Entering a *nested* scoped body (a `while` loop here) while an
/// *enclosing* narrowing sentinel is already present in `scopes`' top frame
/// exercises `narrowing_snapshot`'s non-empty case: the snapshot must
/// actually capture the existing `$narrowed:x` entry (not just find none),
/// so `restore_narrowing` can put it back after the nested body's own
/// isolated processing closes. `is_not_none_narrows_the_body_read_to_an_optional_unwrap`
/// and the leak-isolation test above only ever exercise
/// `narrowing_snapshot` on an *empty* narrowing set (nothing yet narrowed at
/// the point the nested body is entered) -- this is the complementary case,
/// proving the read inside the nested `while` body still sees the *outer*
/// `if`'s narrowing.
#[test]
fn a_nested_scoped_body_entered_while_already_narrowed_still_sees_the_narrowing() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("cond".to_string(), Ty::Bool),
            ("x".to_string(), Ty::Optional(Box::new(Ty::Int))),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![HirStmt::While {
                test: HirExpr::Name("cond".to_string()),
                body: vec![print_x("x")],
            }],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If { body: if_body, .. } = &body[0] else {
        panic!("expected the only function statement to be the lowered `if`");
    };
    let MirStmt::While {
        body: while_body, ..
    } = &if_body[0]
    else {
        panic!("expected the `if` body's only statement to be the lowered `while`");
    };
    assert_eq!(
        while_body[0],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::OptionalUnwrap(
                Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Optional(Box::new(Ty::Int)),
                }),
                Box::new(Ty::Int),
            )],
            ty: Ty::None,
        })
    );
}

/// D-068 review of #780, Finding 1 (MIR side): a reassignment to `None`
/// inside a *nested* `if`'s body must kill the narrowing for every read
/// after the nested `if` closes, within the same enclosing branch -- `if x
/// is not None: (if flag: x = None); print(x)`. Before this fix,
/// `lower_scoped_body`'s snapshot/restore unconditionally reverted
/// `scopes`' narrowing state back to what it was before the nested `if`,
/// discarding the kill and leaving `print(x)` lowered to a
/// `MirExpr::OptionalUnwrap` reading a struct whose `present` field can
/// actually be `false` -- an out-of-bounds/garbage-payload read, not a
/// clean rejection. `pycc_types::check` now rejects the source this HIR
/// models (T0021, via `join_if_branches`'s own analogous fix), so this
/// hand-built-HIR test is the only way to exercise `pycc_mir::build`'s
/// independent copy of the same defect directly.
#[test]
fn a_reassignment_inside_a_nested_if_kills_narrowing_past_the_nested_ifs_own_close() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Optional(Box::new(Ty::Int))),
            ("flag".to_string(), Ty::Bool),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("flag".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::NoneLiteral,
                    }],
                    orelse: vec![],
                },
                print_x("x"),
            ],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the only function statement to be the lowered outer `if`");
    };
    // The kill inside the nested `if flag: x = None` must survive past the
    // nested `if`'s own close: `print(x)` here must read plain
    // `Optional(Int)`, never `OptionalUnwrap`.
    assert_eq!(
        outer_body[1],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        })
    );
}

/// The same defect as above, one mechanism over: a reassignment to `None`
/// inside a nested `while` loop's body must also kill the narrowing for
/// every read after the loop closes, within the same enclosing branch --
/// `if x is not None: (while flag: x = None); print(x)`. Exercises
/// `stmt::lower_loop_body`'s own join between the "loop ran zero times"
/// state (still narrowed) and the "loop ran the body" state (killed),
/// mirroring `pycc_types::join_loop_body`'s identical fix.
#[test]
fn a_reassignment_inside_a_nested_while_kills_narrowing_past_the_loops_own_close() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Optional(Box::new(Ty::Int))),
            ("flag".to_string(), Ty::Bool),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![
                HirStmt::While {
                    test: HirExpr::Name("flag".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::NoneLiteral,
                    }],
                },
                print_x("x"),
            ],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the only function statement to be the lowered outer `if`");
    };
    assert_eq!(
        outer_body[1],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        })
    );
}

/// `.ty()` of an `OptionalUnwrap` node reports the inner type, not
/// `Optional(inner)` -- the read-side mirror of `OptionalWrap`'s own
/// `.ty()` test.
#[test]
fn optional_unwrap_ty_reports_the_inner_type() {
    let expr = MirExpr::OptionalUnwrap(
        Box::new(MirExpr::Name {
            name: "x".to_string(),
            ty: Ty::Optional(Box::new(Ty::Int)),
        }),
        Box::new(Ty::Int),
    );
    assert_eq!(expr.ty(), Ty::Int);
}

/// Issue #769 follow-up (D-068 re-review of #780, third round), soundness
/// blocker 1 (loop re-entry): `if x is not None: while flag: print(x); x =
/// None`. A `while` body is lowered once but executes repeatedly at
/// runtime -- a naive lowering would still see `x` narrowed for the
/// `print(x)` read (it precedes the kill in source order), emitting an
/// unconditional `MirExpr::OptionalUnwrap` that is only safe on the
/// body's first iteration. `pycc_types::check` now rejects the source
/// this HIR models (T0021, via `narrow::apply_kill_prescan`), so this
/// hand-built-HIR test is the only way to exercise `pycc_mir::build`'s
/// independent copy of the same fix (`stmt::lower_loop_body`'s own
/// `apply_kill_prescan` call) directly -- mirroring
/// `a_reassignment_inside_a_nested_while_kills_narrowing_past_the_loops_own_close`
/// above, but proving the fix reaches a read *inside* the loop body
/// itself, not only reads after the loop closes.
#[test]
fn a_while_body_that_reads_then_kills_the_narrowed_name_does_not_unwrap_the_read() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Optional(Box::new(Ty::Int))),
            ("flag".to_string(), Ty::Bool),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![HirStmt::While {
                test: HirExpr::Name("flag".to_string()),
                body: vec![
                    print_x("x"),
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::NoneLiteral,
                    },
                ],
            }],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the only function statement to be the lowered outer `if`");
    };
    let MirStmt::While {
        body: while_body, ..
    } = &outer_body[0]
    else {
        panic!("expected the outer `if` body's only statement to be the lowered `while`");
    };
    assert_eq!(
        while_body[0],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        })
    );
}

/// Same soundness blocker, the `while` *test* variant: `if x is not None:
/// while x < 10: x = None`. The test re-executes on every iteration too,
/// so it must not be lowered as an unconditional `OptionalUnwrap` either
/// -- `stmt::lower_stmt`'s `While` arm applies the kill-prescan before
/// lowering `test`, not only before `lower_loop_body` lowers `body`.
#[test]
fn a_while_test_reading_a_name_the_body_kills_does_not_unwrap_the_test() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Optional(Box::new(Ty::Int)))],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![HirStmt::While {
                test: HirExpr::Compare {
                    op: CmpOpKind::Lt,
                    left: Box::new(HirExpr::Name("x".to_string())),
                    right: Box::new(HirExpr::IntLiteral(10)),
                },
                body: vec![HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::NoneLiteral,
                }],
            }],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the only function statement to be the lowered outer `if`");
    };
    let MirStmt::While { test, .. } = &outer_body[0] else {
        panic!("expected the outer `if` body's only statement to be the lowered `while`");
    };
    assert_eq!(
        test,
        &MirExpr::Compare {
            op: CmpOpKind::Lt,
            left: Box::new(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }),
            right: Box::new(MirExpr::IntLiteral(10)),
            ty: Ty::Bool,
        }
    );
}

/// Completeness guard, matching the checker's own equivalent
/// (`crates/pycc_types/src/tests.rs`): the kill-prescan must not
/// over-drop. A `while` body that reads a narrowed name but never kills
/// it anywhere within the loop must still see it narrowed -- otherwise
/// this fix would trade the soundness bug for a spurious rejection
/// regression (the same "warning" defect class the fast-path-helper fix
/// and the match/try sequencing fix already exist to avoid).
#[test]
fn a_while_body_that_reads_but_never_kills_the_narrowed_name_still_unwraps_the_read() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Optional(Box::new(Ty::Int))),
            ("flag".to_string(), Ty::Bool),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![HirStmt::While {
                test: HirExpr::Name("flag".to_string()),
                body: vec![print_x("x")],
            }],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the only function statement to be the lowered outer `if`");
    };
    let MirStmt::While {
        body: while_body, ..
    } = &outer_body[0]
    else {
        panic!("expected the outer `if` body's only statement to be the lowered `while`");
    };
    assert_eq!(
        while_body[0],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::OptionalUnwrap(
                Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Optional(Box::new(Ty::Int)),
                }),
                Box::new(Ty::Int),
            )],
            ty: Ty::None,
        })
    );
}

/// Issue #769 follow-up (D-068 re-review of #780, third round), soundness
/// blocker 2 (except-from-mid-try): `if x is not None: try: x = None;
/// raise ValueError("boom") except ValueError: print(x)`. The handler is
/// only ever entered after some prefix of the `try` body already
/// executed, so it must not see `x` narrowed the same way the pre-try
/// state saw it -- a naive lowering (starting the handler from the same
/// unmodified pre-try `scopes` snapshot every other isolated body starts
/// from) would still emit `MirExpr::OptionalUnwrap` for the handler's
/// read even though the try body already reassigned `x` to `None` before
/// raising. `pycc_types::check` now rejects the source this HIR models
/// (T0021, via `exception::check_try_stmt`'s own `apply_kill_prescan`
/// call), so this hand-built-HIR test is the only way to exercise
/// `pycc_mir::build`'s independent copy of the same fix
/// (`stmt::lower_stmt`'s `Try` arm) directly.
#[test]
fn an_except_handler_reached_after_a_try_body_kill_does_not_unwrap_the_read() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Optional(Box::new(Ty::Int)))],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![HirStmt::Try {
                body: vec![
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::NoneLiteral,
                    },
                    HirStmt::Raise {
                        exc: Some(HirExpr::Call {
                            callee: "ValueError".to_string(),
                            args: vec![HirExpr::StringLiteral("boom".to_string())],
                        }),
                        cause: None,
                    },
                ],
                handlers: vec![pycc_hir::HirExceptHandler {
                    exc_type: Some(vec!["ValueError".to_string()]),
                    name: None,
                    body: vec![print_x("x")],
                }],
                orelse: vec![],
                finalbody: vec![],
            }],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the only function statement to be the lowered outer `if`");
    };
    let MirStmt::Try { handlers, .. } = &outer_body[0] else {
        panic!("expected the outer `if` body's only statement to be the lowered `try`");
    };
    assert_eq!(
        handlers[0].body[0],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        })
    );
}

/// D-068 re-review of #780 (fourth round, blocker finding 1's MIR half):
/// `except ValueError as x:` reuses the narrowed name `x` as its own `as`
/// binding -- the handler's `bind` call must be paired with a
/// `kill_narrowing` call (`stmt::lower_stmt`'s `Try` arm) exactly like the
/// checker-side fix in `exception::check_try_stmt`
/// (`crates/pycc_types/src/exception.rs`), so a read of `x` inside the
/// handler body lowers to a bare `MirExpr::Name { ty: Instance(..), .. }`
/// -- the caught exception instance's real type -- rather than a stale
/// `MirExpr::OptionalUnwrap` wrapping the pre-handler narrowed `int`. The
/// checker rejects this shape at the type level (T0021, since `print`'s
/// argument is now `Instance(ValueError)` not `Optional[int]`... but this
/// crate trusts the checker already ran, so a direct hand-built-HIR test
/// is the only way to exercise `pycc_mir::build`'s independent copy of the
/// fix.
#[test]
fn an_except_as_binding_reusing_the_narrowed_name_does_not_unwrap_the_handler_read() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Optional(Box::new(Ty::Int)))],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![HirStmt::Try {
                body: vec![HirStmt::Raise {
                    exc: Some(HirExpr::Call {
                        callee: "ValueError".to_string(),
                        args: vec![HirExpr::StringLiteral("boom".to_string())],
                    }),
                    cause: None,
                }],
                handlers: vec![pycc_hir::HirExceptHandler {
                    exc_type: Some(vec!["ValueError".to_string()]),
                    name: Some("x".to_string()),
                    body: vec![print_x("x")],
                }],
                orelse: vec![],
                finalbody: vec![],
            }],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the only function statement to be the lowered outer `if`");
    };
    let MirStmt::Try { handlers, .. } = &outer_body[0] else {
        panic!("expected the outer `if` body's only statement to be the lowered `try`");
    };
    // A read of an exception-instance-typed name lowers to
    // `MirExpr::ExceptionMessage` wrapping the bare `Name` (pre-existing
    // behavior unrelated to this fix -- exception values render via their
    // message, not a direct instance read). The `Instance(ValueError)`
    // type on the wrapped `Name` is what matters here: it proves the read
    // was *not* unwrapped as a stale narrowed `Optional[int]`.
    assert_eq!(
        handlers[0].body[0],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::ExceptionMessage(Box::new(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Instance(Box::new("ValueError".to_string())),
            }))],
            ty: Ty::None,
        })
    );
}

/// D-068 re-review of #780 (rebase onto #542's except* landing): mirrors
/// `an_except_as_binding_reusing_the_narrowed_name_does_not_unwrap_the_handler_read`
/// above for `TryStar` -- `except* ValueError as x:` reuses the narrowed
/// name `x` as its own `as` binding exactly like a plain `except`, so the
/// paired `kill_narrowing` fix in `stmt.rs`'s `TryStar` handler arm needs
/// the same direct hand-built-HIR coverage as the plain `Try` arm it
/// mirrors.
#[test]
fn a_try_star_except_as_binding_reusing_the_narrowed_name_does_not_unwrap_the_handler_read() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Optional(Box::new(Ty::Int)))],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![HirStmt::TryStar {
                body: vec![HirStmt::Raise {
                    exc: Some(HirExpr::Call {
                        callee: "ValueError".to_string(),
                        args: vec![HirExpr::StringLiteral("boom".to_string())],
                    }),
                    cause: None,
                }],
                handlers: vec![pycc_hir::HirExceptHandler {
                    exc_type: Some(vec!["ValueError".to_string()]),
                    name: Some("x".to_string()),
                    body: vec![print_x("x")],
                }],
                orelse: vec![],
                finalbody: vec![],
            }],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the only function statement to be the lowered outer `if`");
    };
    let MirStmt::TryStar { handlers, .. } = &outer_body[0] else {
        panic!("expected the outer `if` body's only statement to be the lowered `try`/`except*`");
    };
    // Same reasoning as the plain-`Try` counterpart above, but `except*`
    // always binds the caught value as `ExceptionGroup` (never the named
    // handler type -- see `stmt.rs`'s `TryStar` handler arm comment). Unlike
    // the builtin exception classes, `ExceptionGroup` is not a registered
    // exception tag (`exception::resolve_exception_tag` doesn't list it),
    // so `rewrite_exception_to_message` leaves the read as a bare `Name`
    // rather than wrapping it in `ExceptionMessage`; what matters here is
    // that the `ty` on that `Name` is the real `Instance(ExceptionGroup)`,
    // not a stale narrowed `Optional[int]`.
    assert_eq!(
        handlers[0].body[0],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Instance(Box::new("ExceptionGroup".to_string())),
            }],
            ty: Ty::None,
        })
    );
}

/// D-068 re-review of #780 (eighth round): mirrors
/// `an_except_handler_reached_after_a_try_body_kill_does_not_unwrap_the_read`
/// above for `TryStar` -- the same handler-reached-after-a-body-kill
/// soundness gap the plain `Try` arm's `pre_handlers_narrowed`/
/// `apply_kill_prescan` sequence fixes applies identically to `except*`,
/// since a subgroup handler also only ever runs after some prefix of the
/// `try` body already executed. `pycc_types::check` now rejects the source
/// this HIR models too (T0021, via `check_try_star_stmt`'s own
/// `apply_kill_prescan` call, added this same round), so this hand-built-HIR
/// test is the only way to exercise `pycc_mir::build`'s independent copy of
/// the fix (`stmt::lower_stmt`'s `TryStar` arm) directly.
#[test]
fn a_try_star_except_handler_reached_after_a_try_body_kill_does_not_unwrap_the_read() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Optional(Box::new(Ty::Int)))],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![HirStmt::TryStar {
                body: vec![
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::NoneLiteral,
                    },
                    HirStmt::Raise {
                        exc: Some(HirExpr::Call {
                            callee: "ValueError".to_string(),
                            args: vec![HirExpr::StringLiteral("boom".to_string())],
                        }),
                        cause: None,
                    },
                ],
                handlers: vec![pycc_hir::HirExceptHandler {
                    exc_type: Some(vec!["ValueError".to_string()]),
                    name: None,
                    body: vec![print_x("x")],
                }],
                orelse: vec![],
                finalbody: vec![],
            }],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the only function statement to be the lowered outer `if`");
    };
    let MirStmt::TryStar { handlers, .. } = &outer_body[0] else {
        panic!("expected the outer `if` body's only statement to be the lowered `try`/`except*`");
    };
    assert_eq!(
        handlers[0].body[0],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        })
    );
}

/// D-068 re-review of #780 (eighth round): proves the `TryStar` arm's
/// `body`/`orelse`/`finalbody` positions route through `lower_scoped_body`
/// (and therefore `lower_stmt_sequence`/`apply_post_if_narrowing`) exactly
/// like the plain `Try` arm's -- an early-return guard inside the `try`
/// body must still narrow the read in the statement right after it, the
/// same early-return-continuation shape
/// `an_early_return_guard_narrows_the_read_after_it_in_a_function_body`
/// proves at the function-body level. Before this round's fix, `TryStar`
/// lowered its body with a raw `.map(lower_stmt)` loop that never called
/// `apply_post_if_narrowing`, so this read would have stayed a plain
/// `MirExpr::Name` (unnarrowed) instead of an `OptionalUnwrap`.
#[test]
fn an_early_return_guard_inside_a_try_star_body_narrows_the_read_after_it() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Optional(Box::new(Ty::Int)))],
        return_ty: Ty::None,
        body: vec![HirStmt::TryStar {
            body: vec![
                HirStmt::If {
                    test: is_none("x"),
                    body: vec![HirStmt::Return(None)],
                    orelse: vec![],
                },
                print_x("x"),
            ],
            handlers: vec![pycc_hir::HirExceptHandler {
                exc_type: Some(vec!["ValueError".to_string()]),
                name: None,
                body: vec![],
            }],
            orelse: vec![],
            finalbody: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::TryStar { body: try_body, .. } = &body[0] else {
        panic!("expected the only function statement to be the lowered `try`/`except*`");
    };
    assert_eq!(
        try_body[1],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::OptionalUnwrap(
                Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Optional(Box::new(Ty::Int)),
                }),
                Box::new(Ty::Int),
            )],
            ty: Ty::None,
        })
    );
}

/// D-068 re-review of #780 (fifth round): a `match` case's own capture
/// pattern (`case x:`) reusing a narrowed name must kill that name's
/// narrowing sentinel too, exactly like the `Try`-handler `as` binding
/// fixed just above -- `lower_match_chain`'s binding loop
/// (`matching.rs`) previously called `bind_variable` alone, never
/// `kill_narrowing`, so a read of the same name *after* the `match`
/// statement (not just inside a case body, which already has its own
/// isolated snapshot/restore) would still wrongly see the pre-match
/// narrowed sentinel and unwrap against the stale narrowed type.
#[test]
fn a_match_capture_pattern_reusing_a_narrowed_name_does_not_unwrap_a_later_read() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Optional(Box::new(Ty::Int))),
            ("y".to_string(), Ty::Optional(Box::new(Ty::Int))),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![
                HirStmt::Match {
                    subject: HirExpr::Name("y".to_string()),
                    cases: vec![pycc_hir::HirMatchCase {
                        pattern: pycc_hir::HirPattern::Capture("x".to_string()),
                        guard: None,
                        body: vec![],
                    }],
                },
                print_x("x"),
            ],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the only function statement to be the lowered outer `if`");
    };
    // `outer_body[0]` is the lowered `match`'s `Seq(assign, chain)`, and
    // `outer_body[1]` is the `print(x)` statement after it -- the read
    // that must see `x`'s real (pattern-capture, still `Optional[int]`)
    // type rather than a stale `OptionalUnwrap`.
    assert_eq!(
        outer_body[1],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        })
    );
}

/// D-068 re-review of #780 (sixth round): a `for` loop's own induction
/// variable reusing a narrowed name must kill that name's narrowing
/// sentinel too, exactly like the `Try`-handler `as` binding and the
/// `match` capture-pattern binding fixed in the two rounds just above --
/// `HirStmt::ForRange`'s arm previously called `bind_variable` alone,
/// never `kill_narrowing`, so a read of the same name *after* the loop
/// closes would still wrongly see the pre-loop narrowed sentinel.
#[test]
fn a_for_range_loop_variable_reusing_a_narrowed_name_does_not_unwrap_a_later_read() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Optional(Box::new(Ty::Int)))],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![
                HirStmt::ForRange {
                    var: "x".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(3),
                    step: HirExpr::IntLiteral(1),
                    body: vec![],
                },
                print_x("x"),
            ],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the only function statement to be the lowered outer `if`");
    };
    // `bind_variable` only inserts when the key is absent (mirroring the
    // checker's own "sticky representation" rule for a compatible
    // reassignment -- `check_assignment`'s `lookup_any`/`Definitely(previous)`
    // path in `pycc_types::lib`), so `x`'s type stays its original
    // `Optional[int]` parameter type rather than becoming `Int` -- what
    // this test actually proves is that the read is a bare `Name`, not a
    // stale `OptionalUnwrap`.
    assert_eq!(
        outer_body[1],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        })
    );
}

/// D-068 re-review of #780 (sixth round): a list comprehension's own
/// result `target` reusing a narrowed name must kill that name's
/// narrowing sentinel too -- `HirStmt::ListCompAssign`'s arm previously
/// called `bind_variable` alone for `target`, never `kill_narrowing`.
#[test]
fn a_list_comprehension_target_reusing_a_narrowed_name_does_not_unwrap_a_later_read() {
    let hir = module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Optional(Box::new(Ty::Int))),
            ("xs".to_string(), Ty::List(Box::new(Ty::Int))),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: is_not_none("x"),
            body: vec![
                HirStmt::ListCompAssign {
                    target: "x".to_string(),
                    var: "i".to_string(),
                    iter: pycc_hir::CompIter::Name("xs".to_string()),
                    cond: None,
                    elt: Box::new(HirExpr::Name("i".to_string())),
                },
                HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                }),
            ],
            orelse: vec![],
        }],
    }]);
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[0] else {
        panic!("expected the only item to be the lowered function");
    };
    let MirStmt::If {
        body: outer_body, ..
    } = &body[0]
    else {
        panic!("expected the only function statement to be the lowered outer `if`");
    };
    // Same `bind_variable` or-insert semantics as the for-range case above:
    // `x`'s recorded type stays its original `Optional[int]` parameter type.
    // What this test proves is that the read is a bare `Name`, not a stale
    // `OptionalUnwrap`.
    assert_eq!(
        outer_body[1],
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Optional(Box::new(Ty::Int)),
            }],
            ty: Ty::None,
        })
    );
}
