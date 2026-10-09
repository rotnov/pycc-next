//! #1298: a caught exception's message `str` is owned by the exception
//! object. Rendering it (`print(e)`, `f"{e}"`) is a borrowed read that every
//! consumer must retain before it consumes, and a `raise` whose message is a
//! borrowed read (`msg`, `obj.s`) must give the exception its own reference.
//!
//! Each case builds MIR by hand, exactly as `pycc_mir` lowers the Python
//! source quoted in its comment, and asserts exact output and exit status:
//! before the fix the release profile's use-after-free exits 0 with wrong
//! output, so success alone proves nothing.

use super::*;
use std::process::Output;

const VALUE_ERROR: u8 = 1;
const KEY_ERROR: u8 = 3;
const EXCEPTION_GROUP: u8 = 24;

fn instance(class: &str) -> Ty {
    Ty::Instance(Box::new(class.to_string()))
}

fn name(n: &str, ty: Ty) -> MirExpr {
    MirExpr::Name {
        name: n.to_string(),
        ty,
    }
}

fn str_name(n: &str) -> MirExpr {
    name(n, Ty::Str)
}

fn lit(s: &str) -> MirExpr {
    MirExpr::StringLiteral(s.to_string())
}

fn concat(a: &str, b: &str) -> MirExpr {
    MirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(lit(a)),
        right: Box::new(lit(b)),
        ty: Ty::Str,
    }
}

/// `print(e)` / `f"{e}"`'s rendered operand: `e`'s message.
fn message_of(binding: &str, class: &str) -> MirExpr {
    MirExpr::ExceptionMessage(Box::new(name(binding, instance(class))))
}

fn print(args: Vec<MirExpr>) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: "print".to_string(),
        args,
        ty: Ty::None,
    })
}

fn call_stmt(callee: &str) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: callee.to_string(),
        args: Vec::new(),
        ty: Ty::None,
    })
}

fn assign(target: &str, value: MirExpr) -> MirStmt {
    MirStmt::Assign {
        target: target.to_string(),
        value,
    }
}

fn value_error(message: MirExpr) -> MirExceptionValue {
    MirExceptionValue::Constructed {
        type_tag: VALUE_ERROR,
        class_name: "ValueError".to_string(),
        message,
    }
}

fn raise(exception: MirExceptionValue, frame: &str) -> MirStmt {
    MirStmt::Raise {
        exception,
        frame_function: frame.to_string(),
    }
}

fn handler(tag: u8, binding: &str, class: &str, body: Vec<MirStmt>) -> MirExceptHandler {
    MirExceptHandler {
        exc_type_tag: Some(vec![tag]),
        binding_name: Some(binding.to_string()),
        binding_ty: Some(instance(class)),
        body,
    }
}

fn try_except(body: Vec<MirStmt>, handler: MirExceptHandler) -> MirStmt {
    MirStmt::Try {
        body,
        handlers: vec![handler],
        orelse: Vec::new(),
        finalbody: Vec::new(),
    }
}

fn function(name: &str, body: Vec<MirStmt>) -> MirItem {
    MirItem::Function {
        name: name.to_string(),
        params: Vec::new(),
        return_ty: Ty::None,
        body,
    }
}

fn module(items: Vec<MirItem>) -> MirModule {
    MirModule {
        items,
        class_defs: Vec::new(),
    }
}

/// Compiles `mir` (debug profile), links it against the runtime and runs
/// it, returning the process output.
fn run(mir: &MirModule, stem: &str) -> Output {
    let dir = pycc_scratch::ScratchDir::new(stem).expect("failed to create scratch dir");
    let obj_path = dir.join(format!("{stem}.o"));
    compile_to_object(mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join(stem);
    link_object_with_runtime(&obj_path, &bin_path);
    Command::new(&bin_path).output().expect("binary should run")
}

fn assert_output(output: &Output, stdout: &str, exit_code: i32) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        stdout,
        "stderr: {stderr}"
    );
    assert_eq!(output.status.code(), Some(exit_code), "stderr: {stderr}");
}

/// ```python
/// try:
///     raise ValueError("boom")
/// except ValueError as e:
///     print(e)
///     print(e)
///     print(e, e)
/// ```
fn print_repeatedly_module() -> MirModule {
    module(vec![MirItem::TopLevelStmt(try_except(
        vec![raise(value_error(lit("boom")), "<module>")],
        handler(
            VALUE_ERROR,
            "e",
            "ValueError",
            vec![
                print(vec![message_of("e", "ValueError")]),
                print(vec![message_of("e", "ValueError")]),
                print(vec![
                    message_of("e", "ValueError"),
                    message_of("e", "ValueError"),
                ]),
            ],
        ),
    ))])
}

// (a)
#[test]
fn printing_a_caught_exception_repeatedly_keeps_its_message_alive() {
    let output = run(&print_repeatedly_module(), "exception_message_print_repeat");
    assert_output(&output, "boom\nboom\nboom boom\n", 0);
}

// IR shape: pins the classification itself, independent of what the
// allocator leaves in freed memory.
#[test]
fn printing_a_caught_exception_retains_the_borrowed_message_before_releasing_it() {
    const READ: &str = "@pycc_rt_exception_message(";
    const INCREF: &str = "@pycc_rt_str_incref(";
    const DECREF: &str = "@pycc_rt_str_decref(";
    let dir = pycc_scratch::ScratchDir::new("exception_message_print_ir")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("exception_message_print_ir.o");
    let mut ir = String::new();
    // D-029: route the `LLVMString` through `llvm_string_to_owned`.
    let mut observer = |module: &inkwell::module::Module<'_>, _| {
        ir = llvm_string_to_owned(module.print_to_string());
    };
    compile_to_object_with_observer(
        &print_repeatedly_module(),
        &obj_path,
        &CompileOptions::default(),
        Some(&mut observer),
    )
    .expect("codegen should succeed");

    // After each message read, the next refcount call on the `str` is its
    // retain, never a release.
    let calls: Vec<&str> = ir
        .lines()
        .filter(|line| line.contains("call "))
        .filter_map(|line| {
            [READ, INCREF, DECREF]
                .into_iter()
                .find(|symbol| line.contains(symbol))
        })
        .collect();
    let reads = calls.iter().filter(|call| **call == READ).count();
    assert_eq!(reads, 4, "four renderings of `e`: {calls:?}");
    for (index, _) in calls.iter().enumerate().filter(|(_, c)| **c == READ) {
        let next_refcount_call = calls[index + 1..].iter().find(|c| **c != READ);
        assert_eq!(
            next_refcount_call,
            Some(&INCREF),
            "a message read must be retained before any release: {calls:?}"
        );
    }
}

// (b)
#[test]
fn fstring_renderings_of_a_caught_exception_keep_its_message_alive() {
    // def render(e: ValueError) -> str:
    //     s: str = f"{e}"
    //     return s
    //
    // try:
    //     raise ValueError("boom")
    // except ValueError as e:
    //     print(f"got {e}")
    //     print(f"got {e}")
    //     print(e)
    //     r: str = render(e)
    //     print(r)
    //     print(e)
    let render = MirItem::Function {
        name: "render".to_string(),
        params: vec![("e".to_string(), instance("ValueError"))],
        return_ty: Ty::Str,
        body: vec![
            assign(
                "s",
                MirExpr::FString(vec![MirFStringPart::Interpolation(Box::new(message_of(
                    "e",
                    "ValueError",
                )))]),
            ),
            MirStmt::Return(Some(str_name("s"))),
        ],
    };
    let got_e = || {
        MirExpr::FString(vec![
            MirFStringPart::Literal("got ".to_string()),
            MirFStringPart::Interpolation(Box::new(message_of("e", "ValueError"))),
        ])
    };
    let mir = module(vec![
        render,
        MirItem::TopLevelStmt(try_except(
            vec![raise(value_error(lit("boom")), "<module>")],
            handler(
                VALUE_ERROR,
                "e",
                "ValueError",
                vec![
                    print(vec![got_e()]),
                    print(vec![got_e()]),
                    print(vec![message_of("e", "ValueError")]),
                    assign(
                        "r",
                        MirExpr::Call {
                            callee: "render".to_string(),
                            args: vec![name("e", instance("ValueError"))],
                            ty: Ty::Str,
                        },
                    ),
                    print(vec![str_name("r")]),
                    print(vec![message_of("e", "ValueError")]),
                ],
            ),
        )),
    ]);
    let output = run(&mir, "exception_message_fstring_reuse");
    assert_output(&output, "got boom\ngot boom\nboom\nboom\nboom\n", 0);
}

// (c)
#[test]
fn a_raise_of_a_local_message_gives_the_exception_its_own_reference() {
    // def g() -> None:
    //     msg: str = "a" + "b"
    //     raise ValueError(msg)
    //
    // try:
    //     g()
    // except ValueError as e:
    //     print(e)
    //     print(e)
    let mir = module(vec![
        function(
            "g",
            vec![
                assign("msg", concat("a", "b")),
                raise(value_error(str_name("msg")), "g"),
            ],
        ),
        MirItem::TopLevelStmt(try_except(
            vec![call_stmt("g")],
            handler(
                VALUE_ERROR,
                "e",
                "ValueError",
                vec![
                    print(vec![message_of("e", "ValueError")]),
                    print(vec![message_of("e", "ValueError")]),
                ],
            ),
        )),
    ]);
    let output = run(&mir, "exception_message_local_raise");
    assert_output(&output, "ab\nab\n", 0);
}

// (d)
#[test]
fn an_exception_group_with_a_local_label_gives_the_group_its_own_reference() {
    // def g() -> None:
    //     label: str = "grp " + "var"
    //     try:
    //         raise ValueError("x")
    //     except ValueError as a:
    //         raise ExceptionGroup(label, [a])
    //
    // try:
    //     g()
    // except* ValueError as eg:
    //     print(eg)
    //     print(eg)
    //
    // pycc renders the group's message without CPython's
    // ` (1 sub-exception)` suffix, a separate pre-existing divergence.
    let group = MirExceptionValue::ConstructedGroup {
        type_tag: EXCEPTION_GROUP,
        class_name: "ExceptionGroup".to_string(),
        message: str_name("label"),
        members: vec![name("a", instance("ValueError"))],
    };
    let mir = module(vec![
        function(
            "g",
            vec![
                assign("label", concat("grp ", "var")),
                try_except(
                    vec![raise(value_error(lit("x")), "g")],
                    handler(VALUE_ERROR, "a", "ValueError", vec![raise(group, "g")]),
                ),
            ],
        ),
        MirItem::TopLevelStmt(MirStmt::TryStar {
            body: vec![call_stmt("g")],
            handlers: vec![handler(
                VALUE_ERROR,
                "eg",
                "ExceptionGroup",
                vec![
                    print(vec![message_of("eg", "ExceptionGroup")]),
                    print(vec![message_of("eg", "ExceptionGroup")]),
                ],
            )],
            orelse: Vec::new(),
            finalbody: Vec::new(),
        }),
    ]);
    let output = run(&mir, "exception_message_group_label");
    assert_output(&output, "grp var\ngrp var\n", 0);
}

// (e)
#[test]
fn an_uncaught_reraise_after_printing_the_binding_renders_the_message() {
    // try:
    //     raise ValueError("boom")
    // except ValueError as e:
    //     print(e)
    //     raise
    let mir = module(vec![MirItem::TopLevelStmt(try_except(
        vec![raise(value_error(lit("boom")), "<module>")],
        handler(
            VALUE_ERROR,
            "e",
            "ValueError",
            vec![print(vec![message_of("e", "ValueError")]), MirStmt::Reraise],
        ),
    ))]);
    let output = run(&mir, "exception_message_uncaught_reraise");
    assert_output(&output, "boom\n", 1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.ends_with("ValueError: boom\n"),
        "unexpected stderr: {stderr}"
    );
}

// (f)
#[test]
fn a_cause_with_a_local_message_owns_its_own_reference() {
    // def g() -> None:
    //     m: str = "cau" + "se"
    //     raise ValueError("top") from KeyError(m)
    //
    // g()
    let mir = module(vec![
        function(
            "g",
            vec![
                assign("m", concat("cau", "se")),
                MirStmt::RaiseFrom {
                    exception: value_error(lit("top")),
                    cause: MirExceptionValue::Constructed {
                        type_tag: KEY_ERROR,
                        class_name: "KeyError".to_string(),
                        message: str_name("m"),
                    },
                    frame_function: "g".to_string(),
                },
            ],
        ),
        MirItem::TopLevelStmt(call_stmt("g")),
    ]);
    let output = run(&mir, "exception_message_local_cause");
    assert_output(&output, "", 1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("KeyError: cause\n"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.ends_with("ValueError: top\n"),
        "unexpected stderr: {stderr}"
    );
}
