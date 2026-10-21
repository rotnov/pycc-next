//! Runtime linking, enum member singletons, classmethods and default values.

use super::*;

// -- #436: NullInstance codegen (class method called through a class) --

/// #436: A `@classmethod` called through a class name (`C.greet(21)`)
/// lowers to MIR with a `MirExpr::NullInstance` as the first argument
/// (the `cls` receiver). This test verifies that codegen emits a null
/// pointer for `NullInstance` and that the resulting binary runs
/// correctly, producing the expected output.
///
///     class C:
///         def __init__(self) -> None:
///             return
///         @classmethod
///         def greet(cls, x: int) -> int:
///             return x * 2
///
///     print(C.greet(21))
///
/// Expected output: `42`.
#[test]
fn null_instance_classmethod_codegens_and_runs() {
    let self_ty = instance_ty("C");
    let init = MirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![MirStmt::Return(None)],
    };
    let greet = MirItem::Function {
        name: "C.greet.classmethod".to_string(),
        params: vec![
            ("cls".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
        ],
        return_ty: Ty::Int,
        body: vec![MirStmt::Return(Some(MirExpr::BinOp {
            op: BinOpKind::Mul,
            left: Box::new(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Int,
            }),
            right: Box::new(MirExpr::IntLiteral(2)),
            ty: Ty::Int,
        }))],
    };
    let mir = MirModule {
        items: vec![
            init,
            greet,
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "C.greet.classmethod".to_string(),
                args: vec![
                    MirExpr::NullInstance { ty: self_ty },
                    MirExpr::IntLiteral(21),
                ],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("null_instance_classmethod")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("null_instance_classmethod.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("null_instance_classmethod");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"42\n");
}

#[test]
fn enum_member_singleton_init_emits_and_runs() {
    // #379: Exercises `emit_enum_member_inits` by building a MIR
    // module with an enum class definition and a top-level statement
    // that reads a member's value. The codegen must emit the
    // per-member singleton init sequence before the top-level
    // statement loop.
    let class_def = pycc_mir::HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Color".to_string(),
        bases: vec![],
        mro: vec!["Color".to_string()],
        attrs: vec![
            ("value".to_string(), Ty::Int),
            ("name".to_string(), Ty::Str),
        ],
        methods: vec![],
        properties: vec![],
        static_methods: vec![],
        class_methods: vec![],
        type_param: None,
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: vec![
            ("RED".to_string(), pycc_mir::EnumMemberValue::Int(1)),
            ("GREEN".to_string(), pycc_mir::EnumMemberValue::Int(2)),
        ],
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "Color.RED.enum_member".to_string(),
                    ty: Ty::Instance(Box::new("Color".to_string())),
                }),
                slot: 0,
                ty: Ty::Int,
            }],
            ty: Ty::None,
        }))],
        class_defs: vec![("Color".to_string(), class_def)],
    };
    let dir =
        pycc_scratch::ScratchDir::new("enum_member_init").expect("failed to create scratch dir");
    let obj_path = dir.join("enum_member_init.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("enum_member_init");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n");
}

#[test]
fn str_valued_enum_member_singleton_init_emits_and_runs() {
    // #892: the `Scalar::Str` arm of `emit_enum_member_inits`. A
    // `str`-valued member's slot 0 holds a runtime string object built by
    // `emit_string_literal`, not a tagged int, and the class's `value`
    // attribute is typed `Ty::Str`. Building and *running* the binary is
    // what proves the emitted init sequence is well-formed -- a codegen-only
    // assertion would not catch a mis-tagged slot word.
    let class_def = pycc_mir::HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "Kind".to_string(),
        bases: vec![],
        mro: vec!["Kind".to_string()],
        attrs: vec![
            ("value".to_string(), Ty::Str),
            ("name".to_string(), Ty::Str),
        ],
        methods: vec![],
        properties: vec![],
        static_methods: vec![],
        class_methods: vec![],
        type_param: None,
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: vec![
            (
                "AXIAL".to_string(),
                pycc_mir::EnumMemberValue::Str("axial".to_string()),
            ),
            (
                "RADIAL".to_string(),
                pycc_mir::EnumMemberValue::Str("radial".to_string()),
            ),
        ],
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let read_member = |member: &str, slot: usize, ty: Ty| {
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: format!("Kind.{member}.enum_member"),
                    ty: Ty::Instance(Box::new("Kind".to_string())),
                }),
                slot,
                ty,
            }],
            ty: Ty::None,
        }))
    };
    let mir = MirModule {
        items: vec![
            read_member("AXIAL", 0, Ty::Str),
            read_member("RADIAL", 0, Ty::Str),
            read_member("AXIAL", 1, Ty::Str),
        ],
        class_defs: vec![("Kind".to_string(), class_def)],
    };
    let dir = pycc_scratch::ScratchDir::new("str_enum_member_init")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_enum_member_init.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_enum_member_init");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"axial\nradial\nAXIAL\n");
}

// -- #380: default_value_for_type covers every Mir Ty variant --------

#[test]
fn default_value_for_type_covers_every_mir_ty_variant() {
    let context = Context::create();
    // Scalar types.
    let _ = default_value_for_type(&context, pycc_mir::Ty::Int);
    let _ = default_value_for_type(&context, pycc_mir::Ty::Bool);
    let _ = default_value_for_type(&context, pycc_mir::Ty::Float);
    let _ = default_value_for_type(&context, pycc_mir::Ty::Str);
    let _ = default_value_for_type(&context, pycc_mir::Ty::None);
    let _ = default_value_for_type(&context, pycc_mir::Ty::Infer);
    let _ = default_value_for_type(&context, pycc_mir::Ty::Param(Box::new("T".to_string())));
    // Container types.
    let _ = default_value_for_type(&context, pycc_mir::Ty::List(Box::new(pycc_mir::Ty::Int)));
    let _ = default_value_for_type(
        &context,
        pycc_mir::Ty::Dict(Box::new((pycc_mir::Ty::Int, pycc_mir::Ty::Str))),
    );
    let _ = default_value_for_type(&context, pycc_mir::Ty::Set(Box::new(pycc_mir::Ty::Int)));
    let _ = default_value_for_type(
        &context,
        pycc_mir::Ty::Instance(Box::new("Foo".to_string())),
    );
    let _ = default_value_for_type(&context, pycc_mir::Ty::Protocol(Box::new("P".to_string())));
    let _ = default_value_for_type(
        &context,
        pycc_mir::Ty::Tuple(Box::new(vec![pycc_mir::Ty::Int, pycc_mir::Ty::Str])),
    );
}

#[test]
fn abstract_method_body_with_non_none_return_emits_default_value() {
    // #380 (PR-20): an abstract method has a `Return(None)` body but
    // a non-`None` declared return type. Codegen must emit a correctly
    // typed default value so the LLVM IR is well-typed. This exercises
    // the `else` branch of the `Return(None)` arm in `emit_stmt`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "Animal.sound".to_string(),
                params: vec![(
                    "self".to_string(),
                    Ty::Instance(Box::new("Animal".to_string())),
                )],
                return_ty: Ty::Str,
                body: vec![MirStmt::Return(None)],
            },
            MirItem::Function {
                name: "main".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(None)],
            },
        ],
        class_defs: vec![(
            "Animal".to_string(),
            pycc_mir::HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Animal".to_string(),
                bases: Vec::new(),
                mro: vec!["Animal".to_string()],
                attrs: Vec::new(),
                methods: vec![("sound".to_string(), "Animal.sound".to_string())],
                type_param: None,
                properties: Vec::new(),
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: vec!["sound".to_string()],
                is_abstract: true,
            },
        )],
    };
    let dir = pycc_scratch::ScratchDir::new("abstract_default_ret")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("abstract_default_ret.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    // The binary is never run — the abstract method is never called.
    // We only need to verify that codegen produces valid LLVM IR.
}

// -- #381: MirStmt::Seq error propagation in emit_stmt ---------------

#[test]
fn calling_an_undefined_function_inside_a_seq_is_rejected() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Seq(vec![
            call_print(1),
            call_user_fn("does_not_exist_in_seq"),
        ]))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("seq_undefined_fn").expect("failed to create scratch dir");
    let obj_path = dir.join("seq_undefined_fn.o");
    let err = compile_to_object(&mir, &obj_path, None, false).expect_err("should be rejected");
    assert!(
        err.contains("does_not_exist_in_seq"),
        "error should name the offending function: {err}"
    );
}

#[test]
fn seq_with_valid_statements_compiles_and_runs() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Seq(vec![
            call_print(10),
            call_print(20),
        ]))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("seq_valid").expect("failed to create scratch dir");
    let obj_path = dir.join("seq_valid.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("seq_valid");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"10\n20\n");
}

// -- #381: None singleton pattern comparison in emit_expr ---------------

#[test]
fn none_singleton_comparison_emits_zero_carrier() {
    // Exercises the `MirExpr::Name { name, ty: Ty::None } if name ==
    // "None"` arm of `emit_expr` (line 1633).  The MIR mirrors what
    // `lower_pattern_conds` produces for `match x: case None:`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "get_none".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "None".to_string(),
                    ty: Ty::None,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: CmpOpKind::Eq,
                    left: Box::new(MirExpr::Call {
                        callee: "get_none".to_string(),
                        args: vec![],
                        ty: Ty::None,
                    }),
                    right: Box::new(MirExpr::Name {
                        name: "None".to_string(),
                        ty: Ty::None,
                    }),
                    ty: Ty::Bool,
                },
                body: vec![call_print(1)],
                orelse: vec![call_print(0)],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("none_singleton_cmp").expect("failed to create scratch dir");
    let obj_path = dir.join("none_singleton_cmp.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("none_singleton_cmp");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n");
}
