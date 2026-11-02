//! Issue #382's exception handling through pycc's own APIs: the type
//! checker's rewrites of `try`/`raise` (`pycc_types`) and codegen's checks of
//! raw MIR exception paths. The programs built and run end to end are in
//! `tests/issue_382_exceptions.rs`.

use pycc_scratch::ScratchDir;

#[test]
fn direct_type_api_rejects_an_unknown_exception_handler() {
    let hir = pycc_hir::HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![pycc_hir::HirItem::TopLevelStmt(pycc_hir::HirStmt::Try {
            body: vec![],
            handlers: vec![pycc_hir::HirExceptHandler {
                exc_type: Some(vec!["UnknownError".to_string()]),
                name: None,
                body: vec![],
            }],
            orelse: vec![],
            finalbody: vec![],
        })],
        type_aliases: vec![],
        imports: vec![],
        class_defs: vec![],
    };
    let err = pycc_types::check(&hir).expect_err("unknown handlers must fail closed");
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not a recognized exception class"));
}

#[test]
fn direct_type_api_rewrites_every_try_and_raise_operand_path() {
    use pycc_hir::{HirExceptHandler, HirExpr, HirItem, HirModule, HirStmt, Ty};

    let builtin = |name: &str, message: &str| HirExpr::Call {
        callee: name.to_string(),
        args: vec![HirExpr::StringLiteral(message.to_string())],
    };
    let type_param = Ty::Param(Box::new("T".to_string()));
    let identity = HirItem::Function {
        name: "identity".to_string(),
        params: vec![("value".to_string(), type_param.clone())],
        return_ty: type_param,
        body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            identity,
            HirItem::TopLevelStmt(HirStmt::Try {
                body: vec![HirStmt::Raise {
                    exc: Some(builtin("ValueError", "body")),
                    cause: None,
                }],
                handlers: vec![HirExceptHandler {
                    exc_type: Some(vec!["ValueError".to_string()]),
                    name: Some("error".to_string()),
                    body: vec![HirStmt::Raise {
                        exc: Some(HirExpr::Name("error".to_string())),
                        cause: Some(builtin("RuntimeError", "cause")),
                    }],
                }],
                orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(0))],
                finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(0))],
            }),
        ],
        type_aliases: vec![],
        imports: vec![],
        class_defs: vec![],
    };
    pycc_types::check_and_resolve(&hir)
        .expect("valid raise operands in every try suite must resolve successfully");
}

fn invalid_generic_class_instantiation() -> pycc_hir::HirExpr {
    pycc_hir::HirExpr::GenericClassInstantiate {
        class: "D".to_string(),
        type_arg: pycc_hir::Ty::Int,
        args: vec![],
    }
}

fn generic_rewrite_fixture(
    stmt: pycc_hir::HirStmt,
    return_ty: pycc_hir::Ty,
    return_expr: pycc_hir::HirExpr,
    extra_params: Vec<(String, pycc_hir::Ty)>,
) -> pycc_hir::HirModule {
    use pycc_hir::{HirClassDef, HirItem, HirStmt, Ty};

    let mut params = vec![("marker".to_string(), Ty::Param(Box::new("T".to_string())))];
    params.extend(extra_params);
    let producer = HirItem::Function {
        name: "produce".to_string(),
        params,
        return_ty,
        body: vec![HirStmt::Return(Some(return_expr))],
    };
    let d_init = HirItem::Function {
        name: "D.__init__".to_string(),
        params: vec![("self".to_string(), Ty::Instance(Box::new("D".to_string())))],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let d_class = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "D".to_string(),
        bases: vec![],
        mro: vec!["D".to_string()],
        attrs: vec![],
        methods: vec![("__init__".to_string(), "D.__init__".to_string())],
        properties: vec![],
        static_methods: vec![],
        class_methods: vec![],
        type_param: None,
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: vec![],
        is_dataclass: false,
        dataclass_fields: vec![],
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: vec![],
        abstract_methods: vec![],
        is_abstract: false,
    };
    pycc_hir::HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![d_init, producer, HirItem::TopLevelStmt(stmt)],
        type_aliases: vec![],
        imports: vec![],
        class_defs: vec![("D".to_string(), d_class)],
    }
}

fn assert_generic_rewrite_error(module: pycc_hir::HirModule) {
    let err = pycc_types::check_and_resolve(&module)
        .expect_err("a non-generic class subscription must fail during rewriting");
    assert_eq!(err.code, "T0042");
}

#[test]
fn direct_type_api_propagates_generic_rewrite_errors_from_structured_suites() {
    use pycc_hir::{CompIter, HirExceptHandler, HirExpr, HirMatchCase, HirPattern, HirStmt, Ty};

    let harmless = || HirStmt::ExprStmt(HirExpr::IntLiteral(0));
    let invalid = || HirStmt::ExprStmt(invalid_generic_class_instantiation());
    let fixture = |stmt| generic_rewrite_fixture(stmt, Ty::Int, HirExpr::IntLiteral(0), vec![]);
    for stmt in [
        HirStmt::Try {
            body: vec![invalid()],
            handlers: vec![],
            orelse: vec![],
            finalbody: vec![],
        },
        HirStmt::Try {
            body: vec![harmless()],
            handlers: vec![HirExceptHandler {
                exc_type: Some(vec!["ValueError".to_string()]),
                name: None,
                body: vec![invalid()],
            }],
            orelse: vec![],
            finalbody: vec![],
        },
        HirStmt::Try {
            body: vec![harmless()],
            handlers: vec![],
            orelse: vec![invalid()],
            finalbody: vec![],
        },
        HirStmt::Try {
            body: vec![harmless()],
            handlers: vec![],
            orelse: vec![],
            finalbody: vec![invalid()],
        },
        HirStmt::Match {
            subject: HirExpr::BoolLiteral(true),
            cases: vec![HirMatchCase {
                pattern: HirPattern::Wildcard,
                guard: None,
                body: vec![invalid()],
            }],
        },
    ] {
        assert_generic_rewrite_error(fixture(stmt));
    }

    let invalid_int_call = HirExpr::Call {
        callee: "produce".to_string(),
        args: vec![invalid_generic_class_instantiation()],
    };
    assert_generic_rewrite_error(fixture(HirStmt::ListCompAssign {
        target: "values".to_string(),
        var: "item".to_string(),
        iter: CompIter::Range {
            start: invalid_int_call,
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::Name("item".to_string())),
    }));
}

#[test]
fn direct_type_api_propagates_generic_rewrite_errors_from_raise_operands() {
    use pycc_hir::{HirExceptHandler, HirExpr, HirStmt, Ty};

    let invalid_message = HirExpr::Call {
        callee: "produce".to_string(),
        args: vec![invalid_generic_class_instantiation()],
    };
    assert_generic_rewrite_error(generic_rewrite_fixture(
        HirStmt::Raise {
            exc: Some(HirExpr::Call {
                callee: "ValueError".to_string(),
                args: vec![invalid_message],
            }),
            cause: None,
        },
        Ty::Str,
        HirExpr::StringLiteral("message".to_string()),
        vec![],
    ));

    let existing_error_call = || HirExpr::Call {
        callee: "produce".to_string(),
        args: vec![
            invalid_generic_class_instantiation(),
            HirExpr::Name("error".to_string()),
        ],
    };
    let handler_fixture = |raised: HirExpr, cause: Option<HirExpr>| {
        generic_rewrite_fixture(
            HirStmt::Try {
                body: vec![HirStmt::Raise {
                    exc: Some(HirExpr::Call {
                        callee: "ValueError".to_string(),
                        args: vec![HirExpr::StringLiteral("primary".to_string())],
                    }),
                    cause: None,
                }],
                handlers: vec![HirExceptHandler {
                    exc_type: Some(vec!["ValueError".to_string()]),
                    name: Some("error".to_string()),
                    body: vec![HirStmt::Raise {
                        exc: Some(raised),
                        cause,
                    }],
                }],
                orelse: vec![],
                finalbody: vec![],
            },
            Ty::Instance(Box::new("ValueError".to_string())),
            HirExpr::Name("saved".to_string()),
            vec![(
                "saved".to_string(),
                Ty::Instance(Box::new("ValueError".to_string())),
            )],
        )
    };
    assert_generic_rewrite_error(handler_fixture(existing_error_call(), None));
    assert_generic_rewrite_error(handler_fixture(
        HirExpr::Name("error".to_string()),
        Some(existing_error_call()),
    ));
}

#[test]
fn direct_type_api_rejects_bare_return_from_a_value_function() {
    let hir = pycc_hir::HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![pycc_hir::HirItem::Function {
            name: "broken".to_string(),
            params: vec![],
            return_ty: pycc_hir::Ty::Int,
            body: vec![pycc_hir::HirStmt::Return(None)],
        }],
        type_aliases: vec![],
        imports: vec![],
        class_defs: vec![],
    };
    let err = pycc_types::check_and_resolve(&hir)
        .expect_err("a bare return cannot satisfy a value-returning signature");
    assert_eq!(err.code, "T0022");
}

#[test]
fn direct_type_api_unrolls_enum_loops_nested_in_try() {
    let source = "\
from enum import Enum
class Color(Enum):
    RED = 1
    GREEN = 2
x = 0
try:
    for color in Color:
        x = x + color.value
except ValueError:
    pass
";
    let module = pycc_parser::parse(source).expect("the enum-in-try fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("the enum-in-try fixture must lower");
    pycc_types::check_and_resolve(&hir)
        .expect("enum loops nested in try suites must unroll through the public type API");
}

fn assert_raw_codegen_error(name: &str, stmt: pycc_mir::MirStmt, expected_message: &str) {
    let dir =
        ScratchDir::new(&format!("382_raw_codegen_{name}")).expect("failed to create scratch dir");
    let mir = pycc_mir::MirModule {
        items: vec![pycc_mir::MirItem::TopLevelStmt(stmt)],
        class_defs: vec![],
    };
    let err = pycc_codegen::compile_to_object(&mir, &dir.join("invalid.o"), None, false)
        .expect_err("invalid raw MIR must fail closed");
    assert!(err.contains(expected_message), "unexpected error: {err}");
}

#[test]
fn raw_mir_exception_paths_are_checked_in_the_dependency_instance() {
    use pycc_mir::{MirExceptionValue, MirExpr, MirStmt};

    assert_raw_codegen_error(
        "raise_message",
        MirStmt::Raise {
            exception: MirExceptionValue::Constructed {
                type_tag: 1,
                class_name: "ValueError".to_string(),
                message: MirExpr::IntLiteral(42),
            },
            frame_function: "test_fn".to_string(),
        },
        "raise message must be a string",
    );
    assert_raw_codegen_error(
        "raise_cause",
        MirStmt::RaiseFrom {
            exception: MirExceptionValue::Constructed {
                type_tag: 1,
                class_name: "ValueError".to_string(),
                message: MirExpr::StringLiteral("error".to_string()),
            },
            cause: MirExceptionValue::Constructed {
                type_tag: 2,
                class_name: "TypeError".to_string(),
                message: MirExpr::IntLiteral(42),
            },
            frame_function: "test_fn".to_string(),
        },
        "raise cause message must be a string",
    );
    assert_raw_codegen_error(
        "existing_value",
        MirStmt::Raise {
            exception: MirExceptionValue::Existing(MirExpr::IntLiteral(42)),
            frame_function: "test_fn".to_string(),
        },
        "must be an exception instance",
    );

    for (name, nested) in [
        (
            "try_body",
            pycc_mir::MirStmt::Try {
                body: vec![undefined_void_call("missing_try_body")],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![],
            },
        ),
        (
            "handler",
            pycc_mir::MirStmt::Try {
                body: vec![MirStmt::NoOp],
                handlers: vec![pycc_mir::MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![undefined_void_call("missing_handler")],
                }],
                orelse: vec![],
                finalbody: vec![],
            },
        ),
        (
            "else",
            pycc_mir::MirStmt::Try {
                body: vec![MirStmt::NoOp],
                handlers: vec![],
                orelse: vec![undefined_void_call("missing_else")],
                finalbody: vec![],
            },
        ),
        (
            "finally",
            pycc_mir::MirStmt::Try {
                body: vec![MirStmt::NoOp],
                handlers: vec![],
                orelse: vec![],
                finalbody: vec![undefined_void_call("missing_finally")],
            },
        ),
        (
            "if_body",
            MirStmt::If {
                test: MirExpr::BoolLiteral(true),
                body: vec![undefined_void_call("missing_if_body")],
                orelse: vec![],
            },
        ),
    ] {
        assert_raw_codegen_error(name, nested, "missing_");
    }

    let dir = ScratchDir::new("382_raw_codegen_bare_except").expect("failed to create scratch dir");
    let mir = pycc_mir::MirModule {
        items: vec![pycc_mir::MirItem::TopLevelStmt(MirStmt::Try {
            body: vec![MirStmt::Raise {
                exception: MirExceptionValue::Constructed {
                    type_tag: 1,
                    class_name: "ValueError".to_string(),
                    message: MirExpr::StringLiteral("error".to_string()),
                },
                frame_function: "test_fn".to_string(),
            }],
            handlers: vec![pycc_mir::MirExceptHandler {
                exc_type_tag: None,
                binding_name: None,
                binding_ty: None,
                body: vec![MirStmt::NoOp],
            }],
            orelse: vec![],
            finalbody: vec![],
        })],
        class_defs: vec![],
    };
    pycc_codegen::compile_to_object(&mir, &dir.join("bare_except.o"), None, false)
        .expect("a bare handler is valid raw MIR");

    let mir = pycc_mir::MirModule {
        items: vec![
            pycc_mir::MirItem::Function {
                name: "none_expr_through_finally".to_string(),
                params: vec![],
                return_ty: pycc_mir::Ty::None,
                body: vec![MirStmt::Try {
                    body: vec![MirStmt::Return(Some(MirExpr::Name {
                        name: "None".to_string(),
                        ty: pycc_mir::Ty::None,
                    }))],
                    handlers: vec![],
                    orelse: vec![],
                    finalbody: vec![MirStmt::NoOp],
                }],
            },
            pycc_mir::MirItem::Function {
                name: "bare_value_return_through_finally".to_string(),
                params: vec![],
                return_ty: pycc_mir::Ty::Int,
                body: vec![MirStmt::Try {
                    body: vec![MirStmt::Return(None)],
                    handlers: vec![],
                    orelse: vec![],
                    finalbody: vec![MirStmt::NoOp],
                }],
            },
            pycc_mir::MirItem::Function {
                name: "statically_unreachable_match_tail".to_string(),
                params: vec![],
                return_ty: pycc_mir::Ty::None,
                body: vec![MirStmt::Unreachable],
            },
        ],
        class_defs: vec![],
    };
    pycc_codegen::compile_to_object(&mir, &dir.join("return_paths.o"), None, false)
        .expect("raw return routing must remain valid in the dependency instance");

    let invalid_top_level_return = pycc_mir::MirModule {
        items: vec![pycc_mir::MirItem::TopLevelStmt(MirStmt::Return(None))],
        class_defs: vec![],
    };
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = pycc_codegen::compile_to_object(
            &invalid_top_level_return,
            &dir.join("invalid_top_level_return.o"),
            None,
            false,
        );
    }));
    assert!(
        panic.is_err(),
        "raw top-level return must trip codegen's defensive frontend invariant"
    );
}

fn undefined_void_call(name: &str) -> pycc_mir::MirStmt {
    pycc_mir::MirStmt::ExprStmt(pycc_mir::MirExpr::Call {
        callee: name.to_string(),
        args: vec![],
        ty: pycc_mir::Ty::None,
    })
}
