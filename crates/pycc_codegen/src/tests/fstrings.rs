//! f-string interpolation.

use super::*;

#[test]
fn compiles_an_f_string_interpolating_an_int_between_literal_parts() {
    // `x = 5; s = f"n={x}!"` -- `s` would hold `"n=5!"`.
    //
    // Deviations from the task brief, both in this test:
    //
    // 1. The brief's own version wrote `pycc_hir::Ty::Int`/`pycc_hir::
    //    Ty::None` -- but `pycc_hir` is not a dependency of this crate
    //    (only `pycc_mir` is, per `Cargo.toml`), and Rust doesn't
    //    resolve an indirect crate's name from a `pub use` re-export
    //    alone (`pycc_mir::Ty` is the exact same type as `pycc_hir::
    //    Ty`, but the bare path `pycc_hir::` itself isn't in scope
    //    here). Fixed to use the plain `Ty` already imported from
    //    `pycc_mir` at this module's own top (`use pycc_mir::{BinOpKind,
    //    CmpOpKind, MirExpr, MirItem, MirModule, MirStmt, Ty};`),
    //    matching every other test in this file.
    //
    // 2. The brief's own version wrapped the f-string in `print(...)`
    //    instead of a plain `Assign`. `emit_stmt`'s own `print()` arm
    //    (a few hundred lines above) only accepts a *single, `Ty::Int`-
    //    typed* argument today -- any other shape, including a single
    //    `Ty::Str`-typed argument, falls through to its own documented
    //    "this print() argument shape is not supported yet (multi-arg /
    //    non-int print lands in Task 10)" panic, confirmed empirically:
    //    with the brief's own `print(f"n={x}!")` shape, this test
    //    failed with exactly that panic instead of proving f-string
    //    codegen itself works. Wiring a `str`-typed argument into
    //    `print` is explicitly Task 10's job (the crate root's own doc
    //    comment on `emit_stmt`, and the plan's own Task 10 scope) --
    //    implementing it here would reach into a later task's scope.
    //    Changed to a plain `Assign` (`s = f"..."`), the same shape
    //    already used by this brief's own next two tests below, so this
    //    test actually exercises `MirExpr::FString`'s own codegen
    //    without depending on unfinished `print` dispatch.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(5),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Literal("n=".to_string()),
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    })),
                    pycc_mir::MirFStringPart::Literal("!".to_string()),
                ]),
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_int").expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_int.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_an_f_string_interpolating_a_float_and_a_bool() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "s".to_string(),
            value: MirExpr::FString(vec![
                pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::FloatLiteral(2.5))),
                pycc_mir::MirFStringPart::Literal(" ".to_string()),
                pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::BoolLiteral(true))),
            ]),
        })],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("fstring_float_bool").expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_float_bool.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_an_f_string_interpolating_a_none_returning_call_as_none() {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "returns_none".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "text".to_string(),
                value: MirExpr::FString(vec![pycc_mir::MirFStringPart::Interpolation(Box::new(
                    MirExpr::Call {
                        callee: "returns_none".to_string(),
                        args: vec![],
                        ty: Ty::None,
                    },
                ))]),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "text".to_string(),
                    ty: Ty::Str,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_none_call_as_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_none_call_as_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("fstring_none_call_as_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"None\n");
}

#[test]
fn compiles_an_f_string_with_only_literal_parts() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "s".to_string(),
            value: MirExpr::FString(vec![pycc_mir::MirFStringPart::Literal(
                "no interpolation".to_string(),
            )]),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_literal_only")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_literal_only.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_an_f_string_interpolating_an_existing_str_value() {
    // `s = "hi"; t = f"{s} there"` -- added beyond the task brief's own
    // three tests above: none of those ever interpolates an
    // already-`str`-typed value, so `to_str`'s `Scalar::Str` passthrough
    // arm (`return v` -- no `pycc_rt_*_to_str` conversion call at all)
    // would otherwise never execute, an uncovered region under this
    // project's 100%-line-and-region coverage gate (D-014). Also
    // exercises `incref_if_str_duplicate`'s true branch for a bare
    // `Name` read inside an interpolation (needed so the f-string's own
    // final decref of every non-literal part doesn't underflow `s`'s
    // refcount below what its own binding still owns).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::StringLiteral("hi".to_string()),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "t".to_string(),
                value: MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "s".to_string(),
                        ty: Ty::Str,
                    })),
                    pycc_mir::MirFStringPart::Literal(" there".to_string()),
                ]),
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_str_passthrough")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_str_passthrough.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn interpolating_a_none_returning_call_in_an_f_string_renders_none_not_false() {
    // `def f() -> None:\n    return` ; `s = f"got: {f()}"` ; `print(s)`
    // -- must print `"got: None"`. Before this fix, a `None`-typed
    // interpolation's placeholder `Scalar::Bool(0)` (see `emit_expr`'s
    // `Call` arm doc comment) flowed straight into `to_str`, which has
    // no way to tell it apart from a genuine `False` -- rendering
    // `"got: False"` instead.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Literal("got: ".to_string()),
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Call {
                        callee: "f".to_string(),
                        args: vec![],
                        ty: Ty::None,
                    })),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "s".to_string(),
                    ty: Ty::Str,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_none_call_renders_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_none_call_renders_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("fstring_none_call_renders_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"got: None\n");
}

#[test]
fn interpolating_a_none_typed_parameter_renders_none() {
    // `render(source())` exercises the same unit carrier through an
    // f-string interpolation of a parameter name rather than `print`'s
    // dedicated `None` path.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "source".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::Function {
                name: "render".to_string(),
                params: vec![("value".to_string(), Ty::None)],
                return_ty: Ty::Str,
                body: vec![MirStmt::Return(Some(MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "value".to_string(),
                        ty: Ty::None,
                    })),
                ])))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "render".to_string(),
                    args: vec![MirExpr::Call {
                        callee: "source".to_string(),
                        args: vec![],
                        ty: Ty::None,
                    }],
                    ty: Ty::Str,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fstring_none_typed_parameter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fstring_none_typed_parameter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("fstring_none_typed_parameter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"None\n");
}
