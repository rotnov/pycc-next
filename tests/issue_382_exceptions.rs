use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

fn build_and_run(dir: &std::path::Path, src_name: &str, source: &str) -> (bool, Vec<u8>, String) {
    let src = write_fixture(dir, src_name, source);
    let out = dir.join(src_name.replace(".py", ""));
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    if !output.status.success() {
        return (
            false,
            Vec::new(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        );
    }
    let run = Command::new(&out).output().unwrap();
    (
        run.status.success(),
        run.stdout.clone(),
        String::from_utf8_lossy(&run.stderr).to_string(),
    )
}

fn check_only(dir: &std::path::Path, src_name: &str, source: &str) -> (bool, String) {
    let src = write_fixture(dir, src_name, source);
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

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

#[test]
fn a_terminal_try_with_match_and_handler_has_no_fallthrough() {
    let dir = ScratchDir::new("382_terminal_match").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "terminal_match.py",
        "def choose(flag: bool) -> int:\n    try:\n        if flag:\n            match flag:\n                case _:\n                    return 1\n        else:\n            return 2\n    except ValueError:\n        return 3\nprint(choose(True))\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"1\n");
}

#[test]
fn an_exhaustive_bool_match_inside_try_has_no_synthetic_fallthrough() {
    let dir = ScratchDir::new("382_exhaustive_match").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "exhaustive_match.py",
        "def choose(flag: bool) -> int:\n    try:\n        match flag:\n            case True:\n                return 1\n            case False:\n                return 2\n    except ValueError:\n        return 3\nprint(choose(True))\nprint(choose(False))\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"1\n2\n");
}

// -- Caught ValueError --

#[test]
fn caught_value_error_builds_and_runs() {
    let dir = ScratchDir::new("382_cve").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "cve.py",
        "try:\n    raise ValueError(\"bad\")\nexcept ValueError:\n    print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught\n");
}

// -- Catch-all Exception --

#[test]
fn catch_all_exception_builds_and_runs() {
    let dir = ScratchDir::new("382_ca").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "ca.py",
        "try:\n    raise ValueError(\"bad\")\nexcept Exception:\n    print(\"caught all\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught all\n");
}

// -- Handler ordering: first matching handler wins --

#[test]
fn handler_ordering_first_match_wins() {
    let dir = ScratchDir::new("382_ho").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "ho.py",
        "try:\n    raise ValueError(\"bad\")\nexcept ValueError:\n    print(\"value\")\nexcept Exception:\n    print(\"generic\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"value\n");
}

// -- Else body runs when no exception --

#[test]
fn else_body_runs_when_no_exception() {
    let dir = ScratchDir::new("382_else").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "else.py",
        "try:\n    print(\"body\")\nexcept ValueError:\n    print(\"handler\")\nelse:\n    print(\"else\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"body\nelse\n");
}

#[test]
fn else_body_can_read_a_binding_created_by_the_successful_try_body() {
    let dir = ScratchDir::new("382_else_bind").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "else_bind.py",
        "try:\n    x = 7\nexcept ValueError:\n    x = 9\nelse:\n    print(x)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"7\n");
}

// -- Finally always runs --

#[test]
fn finally_always_runs_on_success() {
    let dir = ScratchDir::new("382_fin1").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "fin1.py",
        "try:\n    print(\"body\")\nfinally:\n    print(\"finally\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"body\nfinally\n");
}

#[test]
fn finally_runs_after_handler() {
    let dir = ScratchDir::new("382_fin2").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "fin2.py",
        "try:\n    raise ValueError(\"bad\")\nexcept ValueError:\n    print(\"handler\")\nfinally:\n    print(\"finally\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"handler\nfinally\n");
}

#[test]
fn an_exception_from_else_runs_finally_and_reaches_an_outer_handler() {
    let dir = ScratchDir::new("382_else_exc").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "else_exc.py",
        "try:\n    try:\n        pass\n    except ValueError:\n        pass\n    else:\n        raise TypeError(\"else failed\")\n    finally:\n        print(\"cleanup\")\nexcept TypeError:\n    print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"cleanup\ncaught\n");
}

#[test]
fn an_exception_from_handler_runs_finally_and_reaches_an_outer_handler() {
    let dir = ScratchDir::new("382_handler_exc").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "handler_exc.py",
        "try:\n    try:\n        raise ValueError(\"body failed\")\n    except ValueError:\n        raise TypeError(\"handler failed\")\n    finally:\n        print(\"cleanup\")\nexcept TypeError:\n    print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"cleanup\ncaught\n");
}

// -- Uncaught exception exits with non-zero --

#[test]
fn uncaught_exception_exits_nonzero() {
    let dir = ScratchDir::new("382_uncaught").expect("failed to create scratch dir");
    let (ok, _out, err) = build_and_run(&dir, "uncaught.py", "raise ValueError(\"uncaught\")\n");
    assert!(!ok, "expected non-zero exit for uncaught exception");
    assert!(
        err.contains("ValueError"),
        "stderr should contain ValueError: {err}"
    );
    // #707 added a traceback-frame header naming the enclosing function
    // (`<module>` at top level); see `tests/issue_707_traceback_frames.rs`
    // for the full format's dedicated coverage.
    assert_eq!(
        err,
        "Traceback (most recent call last):\n  File \"<compiled>\", in <module>\nValueError: uncaught\n"
    );
}

// -- Division by zero is caught --

#[test]
fn division_by_zero_is_caught() {
    let dir = ScratchDir::new("382_zdiv").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "zdiv.py",
        "try:\n    x = 1 // 0\nexcept ZeroDivisionError:\n    print(\"caught zdiv\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught zdiv\n");
}

// -- Float division by zero is caught --

#[test]
fn float_division_by_zero_is_caught() {
    let dir = ScratchDir::new("382_fzdiv").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "fzdiv.py",
        "try:\n    x = 1.0 / 0.0\nexcept ZeroDivisionError:\n    print(\"caught fzdiv\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught fzdiv\n");
}

// -- Missing dictionary key is caught --

#[test]
fn missing_dict_key_is_caught() {
    let dir = ScratchDir::new("382_key").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "key.py",
        "d = {\"a\": 1}\ntry:\n    v = d[\"b\"]\nexcept KeyError:\n    print(\"caught key\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught key\n");
}

// -- List index out of range is caught --

#[test]
fn list_index_out_of_range_is_caught() {
    let dir = ScratchDir::new("382_idx").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "idx.py",
        "xs = [1, 2, 3]\ntry:\n    v = xs[10]\nexcept IndexError:\n    print(\"caught idx\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught idx\n");
}

// -- raise from (PEP 409) --

#[test]
fn raise_from_builds_and_runs() {
    let dir = ScratchDir::new("382_from").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "from.py",
        "try:\n    raise ValueError(\"bad\") from TypeError(\"cause\")\nexcept ValueError:\n    print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught\n");
}

#[test]
fn raise_from_none_builds_and_runs() {
    // PEP 409: `from None` suppresses implicit context chaining. pycc emits
    // no traceback, so the observable behavior is identical to a plain
    // `raise` -- the handler still catches the primary exception.
    let dir = ScratchDir::new("382_from_none").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "from_none.py",
        "try:\n    try:\n        raise KeyError(\"original\")\n    except KeyError:\n        raise ValueError(\"bad\") from None\nexcept ValueError:\n    print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught\n");
}

#[test]
fn exceptions_while_evaluating_raise_operands_are_not_overwritten() {
    let dir = ScratchDir::new("382_raise_operand_exception").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "raise_operand_exception.py",
        "def fail_message() -> str:\n    raise RuntimeError(\"inner message\")\n    return \"not reached\"\n\ndef fail_cause() -> str:\n    raise KeyError(\"inner cause\")\n    return \"not reached\"\n\ntry:\n    raise ValueError(fail_message())\nexcept RuntimeError:\n    print(\"message preserved\")\n\ntry:\n    raise ValueError(\"outer\") from TypeError(fail_cause())\nexcept KeyError:\n    print(\"cause preserved\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"message preserved\ncause preserved\n");
}

// -- Bare re-raise propagates --

#[test]
fn bare_reraise_propagates() {
    let dir = ScratchDir::new("382_reraise").expect("failed to create scratch dir");
    let (ok, _out, err) = build_and_run(
        &dir,
        "reraise.py",
        "try:\n    raise ValueError(\"orig\")\nexcept ValueError:\n    raise\n",
    );
    assert!(!ok, "expected non-zero exit for re-raised exception");
    assert!(
        err.contains("ValueError"),
        "stderr should contain ValueError: {err}"
    );
}

// -- Bare raise outside handler is rejected --

#[test]
fn bare_raise_outside_handler_is_rejected() {
    let dir = ScratchDir::new("382_bare").expect("failed to create scratch dir");
    let (ok, _combined) = check_only(&dir, "bare.py", "raise\n");
    assert!(!ok, "bare raise outside handler should be rejected");
}

#[test]
fn a_user_function_cannot_silently_shadow_a_builtin_exception_in_raise() {
    let dir = ScratchDir::new("382_shadowed_exception_fn").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "shadowed_exception_fn.py",
        "def ValueError(message: str) -> int:\n    return 1\n\nraise ValueError(\"not builtin\")\n",
    );
    assert!(
        !ok,
        "a shadowing function must not become a builtin exception"
    );
    assert!(
        combined.contains("T0021"),
        "unexpected diagnostic: {combined}"
    );
}

#[test]
fn a_user_class_cannot_silently_shadow_a_builtin_exception_in_raise() {
    let dir =
        ScratchDir::new("382_shadowed_exception_class").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "shadowed_exception_class.py",
        "class ValueError:\n    def __init__(self, message: str):\n        pass\n\nraise ValueError(\"not builtin\")\n",
    );
    assert!(!ok, "a shadowing class must not become a builtin exception");
    assert!(
        combined.contains("T0021"),
        "unexpected diagnostic: {combined}"
    );
}

#[test]
fn a_value_binding_cannot_silently_shadow_a_builtin_exception_in_raise() {
    let dir =
        ScratchDir::new("382_shadowed_exception_value").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "shadowed_exception_value.py",
        "ValueError = 1\nraise ValueError(\"not builtin\")\n",
    );
    assert!(!ok, "a value binding must not become a builtin exception");
    assert!(
        combined.contains("T0021"),
        "unexpected diagnostic: {combined}"
    );
}

#[test]
fn a_function_local_cannot_silently_shadow_a_builtin_exception_in_raise() {
    let dir =
        ScratchDir::new("382_shadowed_exception_local").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "shadowed_exception_local.py",
        "def fail() -> None:\n    raise ValueError(\"not builtin\")\n    ValueError = 1\n\nfail()\n",
    );
    assert!(!ok, "a local binding must not become a builtin exception");
    assert!(
        combined.contains("T0021"),
        "unexpected diagnostic: {combined}"
    );
}

#[test]
fn a_value_binding_cannot_silently_shadow_a_builtin_exception_in_except() {
    let dir =
        ScratchDir::new("382_shadowed_exception_handler").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "shadowed_exception_handler.py",
        "ValueError = 1\ntry:\n    raise RuntimeError(\"boom\")\nexcept ValueError:\n    pass\n",
    );
    assert!(
        !ok,
        "a value binding must not become a builtin handler type"
    );
    assert!(
        combined.contains("T0021"),
        "unexpected diagnostic: {combined}"
    );
}

#[test]
fn a_try_body_binding_cannot_silently_shadow_its_exception_handler_type() {
    let dir = ScratchDir::new("382_shadowed_exception_handler_from_body")
        .expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "shadowed_exception_handler_from_body.py",
        "try:\n    ValueError = 1\n    raise RuntimeError(\"boom\")\nexcept ValueError:\n    pass\n",
    );
    assert!(
        !ok,
        "a try-body binding must not become a builtin handler type"
    );
    assert!(
        combined.contains("T0021"),
        "unexpected diagnostic: {combined}"
    );
}

// -- Return in try body routes through finally --

#[test]
fn return_in_try_routes_through_finally() {
    let dir = ScratchDir::new("382_retfin").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "retfin.py",
        "def f() -> int:\n    try:\n        return 1\n    finally:\n        print(\"cleanup\")\n    return 2\nx = f()\nprint(x)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"cleanup\n1\n");
}

// -- except-as binding is accessible in handler body --

#[test]
fn except_as_binding_is_accessible() {
    let dir = ScratchDir::new("382_asbind").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "asbind.py",
        "try:\n    raise ValueError(\"caught_msg\")\nexcept ValueError as e:\n    saved = e\n    print(\"got it\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"got it\n");
}

// -- Unknown exception type in handler is rejected --

#[test]
fn unknown_exception_type_in_handler_is_rejected() {
    let dir = ScratchDir::new("382_unk").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "unk.py",
        "try:\n    pass\nexcept TypoError:\n    pass\n",
    );
    assert!(!ok, "unknown exception type should be rejected");
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

// -- Unmatched exception propagates to outer handler --

#[test]
fn unmatched_exception_propagates_to_outer_handler() {
    let dir = ScratchDir::new("382_prop").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "prop.py",
        "try:\n    try:\n        raise ValueError(\"inner\")\n    except TypeError:\n        print(\"inner handler\")\nexcept ValueError:\n    print(\"outer handler\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"outer handler\n");
}

// -- Exceptions cross user-function boundaries --

#[test]
fn explicit_raise_in_called_function_is_caught_by_caller() {
    let dir = ScratchDir::new("382_call_raise").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "call_raise.py",
        "def fail() -> None:\n    raise ValueError(\"from callee\")\n\ntry:\n    fail()\nexcept ValueError:\n    print(\"caught caller\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught caller\n");
}

#[test]
fn a_value_returning_function_may_terminate_only_by_raising() {
    let dir = ScratchDir::new("382_raise_int").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "raise_int.py",
        "def fail() -> int:\n    raise ValueError(\"from value callee\")\n\ntry:\n    x = fail()\nexcept ValueError:\n    print(\"caught value callee\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught value callee\n");
}

#[test]
fn a_value_function_with_a_raising_try_finally_cannot_fall_through() {
    let dir = ScratchDir::new("382_raise_finally_int").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "raise_finally_int.py",
        "def fail() -> int:\n    try:\n        raise ValueError(\"from try\")\n    finally:\n        print(\"cleanup\")\n\ntry:\n    x = fail()\nexcept ValueError:\n    print(\"caught value callee\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"cleanup\ncaught value callee\n");
}

#[test]
fn a_generic_call_inside_an_exception_message_is_monomorphized() {
    let dir = ScratchDir::new("382_raise_generic_message").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "raise_generic_message.py",
        "def identity[T](x: T) -> T:\n    return x\n\ndef fail() -> int:\n    raise ValueError(identity(\"nested message\"))\n\ntry:\n    x = fail()\nexcept ValueError:\n    print(\"caught generic message\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught generic message\n");
}

#[test]
fn nested_finally_blocks_preserve_a_value_return() {
    let dir = ScratchDir::new("382_nested_finally_value").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "nested_finally_value.py",
        "def value() -> int:\n    try:\n        try:\n            return 7\n        finally:\n            print(\"inner\")\n    finally:\n        print(\"outer\")\n\nprint(value())\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"inner\nouter\n7\n");
}

#[test]
fn nested_finally_blocks_preserve_a_bare_none_return() {
    let dir = ScratchDir::new("382_nested_finally_none").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "nested_finally_none.py",
        "def finish() -> None:\n    try:\n        try:\n            return\n        finally:\n            print(\"inner\")\n    finally:\n        print(\"outer\")\n\nfinish()\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"inner\nouter\n");
}

#[test]
fn a_none_expression_return_routes_through_finally() {
    let dir = ScratchDir::new("382_none_expr_finally").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "none_expr_finally.py",
        "def side_effect() -> None:\n    print(\"value\")\n\ndef finish() -> None:\n    try:\n        return side_effect()\n    finally:\n        print(\"cleanup\")\n\nfinish()\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"value\ncleanup\n");
}

#[test]
fn runtime_exception_in_called_function_is_caught_by_caller() {
    let dir = ScratchDir::new("382_call_runtime").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "call_runtime.py",
        "def fail() -> None:\n    x = 1 // 0\n    print(\"must not run\")\n\ntry:\n    fail()\nexcept ZeroDivisionError:\n    print(\"caught caller\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught caller\n");
}

// -- An exception transfers control immediately --

#[test]
fn runtime_exception_skips_remaining_try_body() {
    let dir = ScratchDir::new("382_skip_try").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "skip_try.py",
        "try:\n    x = 1 // 0\n    print(\"must not run\")\nexcept ZeroDivisionError:\n    print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught\n");
}

#[test]
fn runtime_exception_skips_remaining_nested_suite() {
    let dir = ScratchDir::new("382_skip_nested").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "skip_nested.py",
        "try:\n    if True:\n        x = 1 // 0\n        print(\"must not run\")\nexcept ZeroDivisionError:\n    print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught\n");
}

#[test]
fn failed_assignment_does_not_overwrite_its_target() {
    let dir = ScratchDir::new("382_atomic_assign").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "atomic_assign.py",
        "x = 5\ntry:\n    x = 1 // 0\nexcept ZeroDivisionError:\n    print(x)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"5\n");
}

#[test]
fn failed_print_argument_produces_no_partial_output() {
    let dir = ScratchDir::new("382_atomic_print").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "atomic_print.py",
        "try:\n    print(1 // 0)\nexcept ZeroDivisionError:\n    print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught\n");
}

#[test]
fn failed_condition_does_not_enter_either_branch() {
    let dir = ScratchDir::new("382_condition").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "condition.py",
        "try:\n    if 1 // 0:\n        print(\"must not run\")\n    else:\n        print(\"must not run either\")\nexcept ZeroDivisionError:\n    print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught\n");
}

#[test]
fn failed_condition_does_not_mutate_in_the_selected_branch() {
    let dir = ScratchDir::new("382_condition_effect").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "condition_effect.py",
        "xs = [1]\ntry:\n    if 1 // 0:\n        xs.append(2)\n    else:\n        xs.append(3)\nexcept ZeroDivisionError:\n    print(len(xs))\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"1\n");
}

#[test]
fn failed_left_operand_skips_the_right_operand() {
    let dir = ScratchDir::new("382_operand_order").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "operand_order.py",
        "def side_effect() -> int:\n    print(\"must not run\")\n    return 2\n\ntry:\n    x = (1 // 0) + side_effect()\nexcept ZeroDivisionError:\n    print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught\n");
}

#[test]
fn uncaught_failed_left_operand_skips_the_right_operand_at_top_level() {
    let dir = ScratchDir::new("382_top_operand").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "top_operand.py",
        "def side_effect() -> int:\n    print(\"must not run\")\n    return 2\n\nx = (1 // 0) + side_effect()\n",
    );
    assert!(!ok, "expected the uncaught exception to exit non-zero");
    assert!(
        out.is_empty(),
        "right operand ran after the exception: {out:?}"
    );
    // #707: `pycc_rt::exception::raise_builtin` (the path a runtime-raised
    // builtin error like this `1 // 0` takes) never calls
    // `pycc_rt_exception_set_frame`, so it carries no recorded frame and
    // must render as a bare one-line message with no `Traceback` header --
    // exactly like the pre-#707 format, and exactly like the unframed
    // `render_single_exception_with_no_frame_omits_traceback_header` unit
    // case in `crates/pycc_rt/src/exception.rs`. Assert the exact stderr
    // rather than only `contains`, so a regression that starts (incorrectly)
    // attaching a frame to this path is caught here.
    assert_eq!(
        err, "ZeroDivisionError: integer division by zero\n",
        "an uncaught runtime-raised builtin error must render without a Traceback header"
    );
}

#[test]
fn failed_return_expression_is_caught_before_return_commits() {
    let dir = ScratchDir::new("382_return_expr").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "return_expr.py",
        "def value() -> int:\n    try:\n        return 1 // 0\n    except ZeroDivisionError:\n        return 7\n    return 9\n\nprint(value())\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"7\n");
}

#[test]
fn nested_handler_does_not_replace_outer_bare_reraise_value() {
    let dir = ScratchDir::new("382_nested_reraise").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "nested_reraise.py",
        "try:\n    try:\n        raise ValueError(\"outer\")\n    except ValueError:\n        try:\n            raise TypeError(\"inner\")\n        except TypeError:\n            pass\n        raise\nexcept ValueError:\n    print(\"outer preserved\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"outer preserved\n");
}

#[test]
fn raising_an_except_binding_preserves_its_exception_type() {
    let dir = ScratchDir::new("382_raise_binding").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "raise_binding.py",
        "try:\n    try:\n        raise ValueError(\"original\")\n    except ValueError as e:\n        raise e\nexcept ValueError:\n    print(\"type preserved\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"type preserved\n");
}

#[test]
fn raise_from_an_except_binding_preserves_the_primary_exception_type() {
    let dir = ScratchDir::new("382_raise_from_binding").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "raise_from_binding.py",
        "try:\n    try:\n        raise ValueError(\"original\")\n    except ValueError as e:\n        raise e from TypeError(\"cause\")\nexcept ValueError:\n    print(\"primary preserved\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"primary preserved\n");
}

#[test]
fn exception_in_finally_overrides_pending_return() {
    let dir = ScratchDir::new("382_finally_raise").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "finally_raise.py",
        "def fail() -> int:\n    try:\n        return 1\n    finally:\n        raise ValueError(\"override\")\n    return 2\n\ntry:\n    x = fail()\nexcept ValueError:\n    print(\"caught override\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught override\n");
}

#[test]
fn finally_runs_while_an_unmatched_exception_is_propagating() {
    let dir = ScratchDir::new("382_finally_pending").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "finally_pending.py",
        "try:\n    try:\n        raise ValueError(\"pending\")\n    finally:\n        print(\"cleanup\")\nexcept ValueError:\n    print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"cleanup\ncaught\n");
}

#[test]
fn return_in_finally_is_rejected_at_build_time() {
    // PEP 765 (#738, Part 1 of #543): a `return` in a `finally` block used
    // to build and run here, silently suppressing the pending
    // `ValueError` -- this test used to assert exactly that permissive
    // runtime behavior. It is now rejected outright at build time, so the
    // build itself fails with `L0001` and the binary never runs. The
    // underlying runtime mechanism this test used to exercise (a `return`
    // reached through a `finally` overriding a pending exception) remains
    // covered at the codegen level by `crates/pycc_codegen/src/tests.rs`
    // (e.g. `try_finally_body_with_return_does_not_fall_through`), which
    // constructs the HIR directly rather than going through this compiler's
    // own AST-lowering front end -- exactly the seam this PEP 765 check
    // lives at.
    let dir = ScratchDir::new("382_finally_return").expect("failed to create scratch dir");
    let (ok, _out, err) = build_and_run(
        &dir,
        "finally_return.py",
        "def value() -> int:\n    try:\n        raise ValueError(\"suppressed\")\n    finally:\n        return 7\n    return 9\n\nprint(value())\n",
    );
    assert!(!ok, "build should fail for a `return` in a `finally` block");
    assert!(
        err.contains("L0001"),
        "expected an L0001 diagnostic, got: {err}"
    );
    assert!(
        err.contains("'return' in a 'finally' block"),
        "expected the PEP 765 finally-return message, got: {err}"
    );
}

#[test]
fn an_exception_class_crosses_a_project_import_boundary() {
    // #898 (Part 1 of #881): the builtin exception hierarchy is seeded per
    // module but reconciled program-wide by `pycc_hir::link`, and the
    // runtime type tags are assigned once over the linked program -- so a
    // class declared in one file is raisable and catchable in another.
    let dir = ScratchDir::new("382_project_import").expect("failed to create scratch dir");
    write_fixture(
        &dir,
        "errors.py",
        "class AppError(ValueError):\n    pass\n\n\nclass OtherError(ValueError):\n    pass\n",
    );
    let (ok, out, err) = build_and_run(
        &dir,
        "main.py",
        "from errors import AppError, OtherError\n\n\
         try:\n    raise AppError(\"boom\")\nexcept OtherError:\n    print(\"wrong\")\n\
         except AppError:\n    print(\"caught\")\n\
         except ValueError:\n    print(\"too broad\")\n",
    );
    assert!(ok, "the two-file program must build and run: {err}");
    assert_eq!(String::from_utf8_lossy(&out).trim(), "caught");
}
