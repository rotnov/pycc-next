//! #1316: a foreign `object` operation inside a function body, and Part 1
//! of #1096: the same operation inside a module-level `try`.
//!
//! Every test compiles real MIR as an `ext` object -- LLVM's verifier runs
//! before any assertion here is believed -- and reads the LLVM text of the
//! user function `f` (mangled `pyfn_f`) or of the module-exec entry. The
//! module-exec edge *outside* every `try` keeps its own tests in
//! `foreign_attr.rs`, `foreign_call.rs` and `foreign_len.rs`, which pin
//! that it is emitted unchanged; the tests at the end of this file pin
//! where it changes.

use crate::{
    CompileOptions, EXT_MODULE_EXEC_SYMBOL, EXT_NAME_ERROR_SYMBOL, EXT_OBJ_ERROR_BRIDGE_SYMBOL,
    compile_to_object_with_observer,
};
use inkwell::values::AnyValue;
use pycc_mir::{BinOpKind, MirExceptHandler, MirExpr, MirItem, MirModule, MirStmt, Ty};

/// `import copy` -- the foreign module global every test reads.
fn import_copy() -> MirItem {
    MirItem::ForeignImport {
        local_name: "copy".to_string(),
        module_path: "copy".to_string(),
        from: None,
    }
}

fn copy_name() -> MirExpr {
    MirExpr::Name {
        name: "copy".to_string(),
        ty: Ty::Object,
    }
}

fn copy_boxed() -> Box<MirExpr> {
    Box::new(copy_name())
}

/// `def f(<params>) -> <return_ty>: <body>`, ahead of any `extra` items.
fn function(params: Vec<(String, Ty)>, return_ty: Ty, body: Vec<MirStmt>) -> MirItem {
    MirItem::Function {
        name: "f".to_string(),
        params,
        return_ty,
        body,
    }
}

/// The LLVM text of the named functions after compiling `items` as an
/// `ext` object, in the order asked for.
fn functions_ir(label: &str, items: Vec<MirItem>, names: &[&str]) -> Vec<String> {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut irs = vec![String::new(); names.len()];
    let mut observer = |module: &inkwell::module::Module<'_>, _: Option<&'static str>| {
        for (slot, name) in irs.iter_mut().zip(names) {
            if let Some(function) = module.get_function(name) {
                *slot = crate::llvm_string_to_owned(function.print_to_string());
            }
        }
    };
    compile_to_object_with_observer(
        &MirModule {
            items,
            ..Default::default()
        },
        &dir.join(format!("{label}.o")),
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
        Some(&mut observer),
    )
    .expect("ext codegen should succeed");
    for (ir, name) in irs.iter().zip(names) {
        assert!(!ir.is_empty(), "no {name} was emitted");
    }
    irs
}

/// The LLVM text of `f` after compiling `import copy` plus `items`.
fn f_ir(label: &str, items: Vec<MirItem>) -> String {
    let mut all = vec![import_copy()];
    all.extend(items);
    functions_ir(label, all, &["pyfn_f"]).remove(0)
}

/// `f`, with one statement discarding `expr`.
fn discarding(expr: MirExpr) -> Vec<MirItem> {
    vec![function(
        Vec::new(),
        Ty::None,
        vec![MirStmt::ExprStmt(expr), MirStmt::Return(None)],
    )]
}

/// The body of the block labelled `label` in `ir`, up to its terminator.
fn block<'a>(ir: &'a str, label: &str) -> &'a str {
    let header = format!("\n{label}:");
    let start = ir
        .find(&header)
        .unwrap_or_else(|| panic!("no `{label}` block:\n{ir}"))
        + header.len();
    let rest = &ir[start..];
    let end = rest.find("\n\n").unwrap_or(rest.len());
    &rest[..end]
}

/// The `{label}_fail` block bridges CPython's exception and branches to
/// `f`'s own `exception_exit`, and nothing in `f` returns the module-exec
/// status.
fn assert_bridged_to_exit(ir: &str, label: &str) {
    let fail = block(ir, &format!("{label}_fail"));
    assert!(
        fail.contains(&format!("call i32 @{EXT_OBJ_ERROR_BRIDGE_SYMBOL}()")),
        "{label}_fail must bridge CPython's exception:\n{ir}"
    );
    assert!(
        fail.trim_end().ends_with("br label %exception_exit"),
        "{label}_fail must branch to the function's exception exit:\n{ir}"
    );
    assert!(!ir.contains("ret i64 -1"), "{ir}");
}

#[test]
fn a_foreign_attribute_load_in_a_function_bridges_to_the_exception_exit() {
    let ir = f_ir(
        "foreign_fail_attr",
        discarding(MirExpr::ObjAttrGet {
            base: copy_boxed(),
            attr: "deepcopy".to_string(),
            ty: Ty::Object,
        }),
    );
    assert_bridged_to_exit(&ir, "foreign_attr");
}

/// A method call has two failure points -- the lookup and the call -- and
/// each takes the function edge.
#[test]
fn a_foreign_method_call_in_a_function_bridges_both_of_its_failure_points() {
    let ir = f_ir(
        "foreign_fail_method",
        discarding(MirExpr::ObjMethodCall {
            base: copy_boxed(),
            method: "copy".to_string(),
            args: vec![MirExpr::IntLiteral(1)],
        }),
    );
    assert_bridged_to_exit(&ir, "foreign_call_lookup");
    assert_bridged_to_exit(&ir, "foreign_call");
}

#[test]
fn a_direct_foreign_call_in_a_function_bridges_to_the_exception_exit() {
    let ir = f_ir(
        "foreign_fail_direct_call",
        discarding(MirExpr::ObjCall {
            callee: copy_boxed(),
            args: Vec::new(),
        }),
    );
    assert_bridged_to_exit(&ir, "foreign_call");
}

#[test]
fn a_foreign_subscript_in_a_function_bridges_to_the_exception_exit() {
    let ir = f_ir(
        "foreign_fail_subscript",
        discarding(MirExpr::ObjSubscript {
            base: copy_boxed(),
            index: Box::new(MirExpr::IntLiteral(0)),
        }),
    );
    assert_bridged_to_exit(&ir, "foreign_subscript");
}

#[test]
fn a_foreign_len_in_a_function_bridges_to_the_exception_exit() {
    let ir = f_ir(
        "foreign_fail_len",
        discarding(MirExpr::ObjLen { base: copy_boxed() }),
    );
    assert_bridged_to_exit(&ir, "foreign_len");
}

/// The four explicit conversions share one emitter shape each; every one
/// takes the function edge.
#[test]
fn each_foreign_conversion_in_a_function_bridges_to_the_exception_exit() {
    for (callee, ty, label) in [
        ("float", Ty::Float, "foreign_to_float"),
        ("int", Ty::Int, "foreign_to_int"),
        ("str", Ty::Str, "foreign_to_str"),
        ("bool", Ty::Bool, "foreign_truthy"),
    ] {
        let ir = f_ir(
            &format!("foreign_fail_conv_{callee}"),
            discarding(MirExpr::Call {
                callee: callee.to_string(),
                args: vec![copy_name()],
                ty,
            }),
        );
        assert_bridged_to_exit(&ir, label);
    }
}

/// A condition-position truth test has no expression guard after it at
/// all, which is one of the two reasons the branch is immediate.
#[test]
fn a_foreign_condition_in_a_function_bridges_to_the_exception_exit() {
    let ir = f_ir(
        "foreign_fail_if",
        vec![function(
            Vec::new(),
            Ty::None,
            vec![
                MirStmt::If {
                    test: copy_name(),
                    body: vec![MirStmt::Return(None)],
                    orelse: Vec::new(),
                },
                MirStmt::Return(None),
            ],
        )],
    );
    assert_bridged_to_exit(&ir, "foreign_truthy");
}

/// Inside a `try`, the innermost exception target is the handler dispatch,
/// not the function's exit: the foreign failure is catchable.
#[test]
fn a_foreign_failure_inside_a_try_branches_to_the_handler_not_the_exit() {
    let ir = f_ir(
        "foreign_fail_try",
        vec![function(
            Vec::new(),
            Ty::None,
            vec![
                MirStmt::Try {
                    body: vec![MirStmt::ExprStmt(MirExpr::ObjAttrGet {
                        base: copy_boxed(),
                        attr: "missing".to_string(),
                        ty: Ty::Object,
                    })],
                    handlers: vec![MirExceptHandler {
                        exc_type_tag: None,
                        binding_name: None,
                        binding_ty: None,
                        body: vec![MirStmt::Return(None)],
                    }],
                    orelse: Vec::new(),
                    finalbody: Vec::new(),
                },
                MirStmt::Return(None),
            ],
        )],
    );
    let fail = block(&ir, "foreign_attr_fail");
    assert!(
        fail.contains(&format!("call i32 @{EXT_OBJ_ERROR_BRIDGE_SYMBOL}()")),
        "{ir}"
    );
    let terminator = fail.trim_end().lines().last().unwrap_or_default().trim();
    assert!(terminator.starts_with("br label %"), "{ir}");
    assert_ne!(terminator, "br label %exception_exit", "{ir}");
}

/// A live bigint temporary -- `a * b`, evaluated before the foreign `len`
/// on the right of `+` -- is released on the foreign failure edge before
/// the branch, exactly as `guard_statement_effects` releases it.
#[test]
fn a_foreign_failure_releases_a_pending_bigint_temporary_before_branching() {
    let ir = f_ir(
        "foreign_fail_bigint",
        vec![function(
            vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int)],
            Ty::Int,
            vec![MirStmt::Return(Some(MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(MirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(MirExpr::Name {
                        name: "a".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::Name {
                        name: "b".to_string(),
                        ty: Ty::Int,
                    }),
                    ty: Ty::Int,
                }),
                right: Box::new(MirExpr::ObjLen { base: copy_boxed() }),
                ty: Ty::Int,
            }))],
        )],
    );
    // The release is a null-checked call, so the unwind spans the fail
    // block and the release blocks it falls into, up to the one branch to
    // the exit.
    let start = ir
        .find("\nforeign_len_fail:")
        .unwrap_or_else(|| panic!("no foreign_len_fail block:\n{ir}"));
    let unwind = &ir[start..];
    let unwind = &unwind[..unwind
        .find("br label %exception_exit")
        .unwrap_or_else(|| panic!("the unwind must reach the exception exit:\n{ir}"))];
    let bridge = unwind
        .find(&format!("call i32 @{EXT_OBJ_ERROR_BRIDGE_SYMBOL}()"))
        .unwrap_or_else(|| panic!("the failure must be bridged:\n{ir}"));
    let release = unwind
        .find("call void @pycc_rt_bigint_release")
        .unwrap_or_else(|| panic!("the pending temporary must be released:\n{ir}"));
    assert!(bridge < release, "bridge first, then unwind:\n{ir}");
    assert!(!ir.contains("ret i64 -1"), "{ir}");
}

/// A function body reads a foreign global through its initialized flag,
/// because D-041 cannot prove call order; the unbound branch raises a
/// catchable `NameError` naming the local binding.
#[test]
fn an_unbound_foreign_global_read_in_a_function_raises_name_error() {
    let ir = f_ir(
        "foreign_fail_name_error",
        discarding(MirExpr::ObjAttrGet {
            base: copy_boxed(),
            attr: "deepcopy".to_string(),
            ty: Ty::Object,
        }),
    );
    let unbound = block(&ir, "global_unbound");
    assert!(
        unbound.contains(&format!(
            "@{EXT_NAME_ERROR_SYMBOL}(ptr @pycc_foreign_name_copy, i64 4)"
        )),
        "{ir}"
    );
    assert!(
        unbound.trim_end().ends_with("br label %exception_exit"),
        "{ir}"
    );
    assert!(!unbound.contains("llvm.trap"), "{ir}");
}

/// Two reads in one module share one `pycc_ext_name_error` declaration,
/// and each unbound branch still calls it.
#[test]
fn two_unbound_foreign_reads_share_one_name_error_declaration() {
    let read = || {
        MirStmt::ExprStmt(MirExpr::ObjAttrGet {
            base: copy_boxed(),
            attr: "deepcopy".to_string(),
            ty: Ty::Object,
        })
    };
    let ir = f_ir(
        "foreign_fail_name_error_twice",
        vec![function(
            Vec::new(),
            Ty::None,
            vec![read(), read(), MirStmt::Return(None)],
        )],
    );
    let call = format!("call void @{EXT_NAME_ERROR_SYMBOL}(ptr @pycc_foreign_name_copy");
    assert_eq!(ir.matches(&call).count(), 2, "{ir}");
    // A second `add_function` of the same symbol would be renamed by LLVM
    // to `@pycc_ext_name_error.1`; both calls naming the bare symbol is
    // what proves the declaration is reused.
    let renamed = format!("@{EXT_NAME_ERROR_SYMBOL}.");
    assert!(!ir.contains(&renamed), "{ir}");
}

/// Only a foreign `object` read changes: an unbound `int` global read in a
/// function keeps its `llvm.trap`, and so does a module-body read, which
/// D-041 proves bound (it emits no flag check at all there).
#[test]
fn a_non_object_unbound_read_keeps_its_trap_and_the_module_body_is_unchanged() {
    let items = vec![
        import_copy(),
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: "n".to_string(),
            value: MirExpr::IntLiteral(3),
        }),
        function(
            Vec::new(),
            Ty::Int,
            vec![MirStmt::Return(Some(MirExpr::Name {
                name: "n".to_string(),
                ty: Ty::Int,
            }))],
        ),
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ObjAttrGet {
            base: copy_boxed(),
            attr: "deepcopy".to_string(),
            ty: Ty::Object,
        })),
    ];
    let irs = functions_ir(
        "foreign_fail_trap_kept",
        items,
        &["pyfn_f", EXT_MODULE_EXEC_SYMBOL],
    );
    let (f, entry) = (&irs[0], &irs[1]);
    let unbound = block(f, "global_unbound");
    assert!(unbound.contains("llvm.trap"), "{f}");
    assert!(!f.contains(EXT_NAME_ERROR_SYMBOL), "{f}");
    assert!(!entry.contains(EXT_NAME_ERROR_SYMBOL), "{entry}");
    assert!(!entry.contains(EXT_OBJ_ERROR_BRIDGE_SYMBOL), "{entry}");
    assert!(entry.contains("ret i64 -1"), "{entry}");
}

/// The LLVM text of the module-exec entry after compiling `import copy`
/// plus `items` as an `ext` object.
fn entry_ir(label: &str, items: Vec<MirItem>) -> String {
    let mut all = vec![import_copy()];
    all.extend(items);
    functions_ir(label, all, &[EXT_MODULE_EXEC_SYMBOL]).remove(0)
}

/// `copy.missing`, discarded: the foreign failure every module-body test
/// below places somewhere different.
fn missing_attr() -> MirStmt {
    MirStmt::ExprStmt(MirExpr::ObjAttrGet {
        base: copy_boxed(),
        attr: "missing".to_string(),
        ty: Ty::Object,
    })
}

/// `print(<n>)`, so no block in the IR is empty.
fn print_stmt(n: i64) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: "print".to_string(),
        args: vec![MirExpr::IntLiteral(n)],
        ty: Ty::None,
    })
}

/// A module-level `try` with one bare `except:` handler.
fn module_try(
    body: Vec<MirStmt>,
    handler: Vec<MirStmt>,
    orelse: Vec<MirStmt>,
    finalbody: Vec<MirStmt>,
) -> MirItem {
    MirItem::TopLevelStmt(MirStmt::Try {
        body,
        handlers: vec![MirExceptHandler {
            exc_type_tag: None,
            binding_name: None,
            binding_ty: None,
            body: handler,
        }],
        orelse,
        finalbody,
    })
}

/// The label `{label}_fail` branches to after bridging, with LLVM's
/// uniquing digits stripped; panics unless the block bridges and does not
/// return the module-exec status.
fn bridged_target(ir: &str, label: &str) -> String {
    let fail = block(ir, &format!("{label}_fail"));
    assert!(
        fail.contains(&format!("call i32 @{EXT_OBJ_ERROR_BRIDGE_SYMBOL}()")),
        "{label}_fail must bridge CPython's exception:\n{ir}"
    );
    assert!(!fail.contains("ret i64"), "{ir}");
    let terminator = fail.trim_end().lines().last().unwrap_or_default().trim();
    terminator
        .strip_prefix("br label %")
        .unwrap_or_else(|| panic!("{label}_fail must branch: {terminator}\n{ir}"))
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_string()
}

/// Part 1 of #1096: a foreign failure in a module-level `try` body is
/// bridged to the handler dispatch, so the `except` runs as in CPython.
#[test]
fn a_module_try_body_foreign_failure_bridges_to_the_handler_dispatch() {
    let ir = entry_ir(
        "module_fail_try_body",
        vec![module_try(
            vec![missing_attr()],
            vec![print_stmt(1)],
            Vec::new(),
            Vec::new(),
        )],
    );
    assert_eq!(bridged_target(&ir, "foreign_attr"), "try_handler_dispatch");
}

/// In a handler body the innermost target is the `try`'s finally block.
#[test]
fn a_module_handler_body_foreign_failure_bridges_to_the_finally_block() {
    let ir = entry_ir(
        "module_fail_handler",
        vec![module_try(
            vec![print_stmt(1)],
            vec![missing_attr()],
            Vec::new(),
            Vec::new(),
        )],
    );
    assert_eq!(bridged_target(&ir, "foreign_attr"), "try_finally");
}

/// The `else` body is a distinct push site: it skips the same `try`'s
/// handlers and goes to its finally block.
#[test]
fn a_module_else_body_foreign_failure_bridges_to_the_finally_block() {
    let ir = entry_ir(
        "module_fail_else",
        vec![module_try(
            vec![print_stmt(1)],
            vec![print_stmt(2)],
            vec![missing_attr()],
            Vec::new(),
        )],
    );
    assert_eq!(bridged_target(&ir, "foreign_attr"), "try_finally");
}

/// A `finally` body's failure goes to the finally-exception target.
#[test]
fn a_module_finally_body_foreign_failure_bridges_to_the_finally_exception_target() {
    let ir = entry_ir(
        "module_fail_finally",
        vec![module_try(
            vec![print_stmt(1)],
            vec![print_stmt(2)],
            Vec::new(),
            vec![missing_attr()],
        )],
    );
    assert_eq!(bridged_target(&ir, "foreign_attr"), "try_finally_exception");
}

/// With no enclosing `try` -- here inside a top-level `if` -- the edge is
/// unchanged: the `_fail` block returns the module-exec status. Asserted
/// on the block itself only, since another item could declare the bridge.
#[test]
fn a_module_failure_outside_every_try_keeps_the_direct_return() {
    let ir = entry_ir(
        "module_fail_if",
        vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::BoolLiteral(true),
            body: vec![missing_attr()],
            orelse: Vec::new(),
        })],
    );
    let fail = block(&ir, "foreign_attr_fail");
    assert!(!fail.contains(EXT_OBJ_ERROR_BRIDGE_SYMBOL), "{ir}");
    assert!(fail.trim_end().ends_with("ret i64 -1"), "{ir}");
}

/// After the `try` closes, a later top-level failure is back on the
/// direct edge: the recorded exit is innermost again.
#[test]
fn a_module_failure_after_a_try_keeps_the_direct_return() {
    let ir = entry_ir(
        "module_fail_after_try",
        vec![
            module_try(
                vec![print_stmt(1)],
                vec![print_stmt(2)],
                Vec::new(),
                Vec::new(),
            ),
            MirItem::TopLevelStmt(missing_attr()),
        ],
    );
    let fail = block(&ir, "foreign_attr_fail");
    assert!(fail.trim_end().ends_with("ret i64 -1"), "{ir}");
}

/// A foreign `for` inside a module-level `try`: both of the loop's
/// failure blocks -- `iter()` and a raising `__next__` -- bridge to the
/// handler dispatch, the second through the loop's cleanup block, which
/// releases the iterator first (Part 3 of #1092).
#[test]
fn a_module_try_foreign_for_loop_bridges_both_failure_points() {
    let ir = entry_ir(
        "module_fail_for",
        vec![module_try(
            vec![MirStmt::ForObject {
                var: "x".to_string(),
                iter: copy_name(),
                body: vec![print_stmt(1)],
            }],
            vec![print_stmt(2)],
            Vec::new(),
            Vec::new(),
        )],
    );
    assert_eq!(
        bridged_target(&ir, "foreign_iter_get"),
        "try_handler_dispatch"
    );
    let next_fail = block(&ir, "foreign_iter_next_fail");
    assert!(
        next_fail.contains(&format!("call i32 @{EXT_OBJ_ERROR_BRIDGE_SYMBOL}()")),
        "{ir}"
    );
    assert!(
        next_fail
            .trim_end()
            .ends_with("br label %foreign_iter_cleanup"),
        "{ir}"
    );
    let cleanup = block(&ir, "foreign_iter_cleanup");
    assert!(
        cleanup.contains("call void @pycc_ext_obj_release(ptr %foreign_iter_get)"),
        "{ir}"
    );
    assert!(
        cleanup
            .trim_end()
            .ends_with("br label %try_handler_dispatch"),
        "{ir}"
    );
}

/// A live bigint temporary in a module-level `try` is released on the
/// bridged edge before the branch, exactly as in a function body.
#[test]
fn a_module_try_foreign_failure_releases_a_pending_bigint_temporary() {
    let int_name = |name: &str| {
        Box::new(MirExpr::Name {
            name: name.to_string(),
            ty: Ty::Int,
        })
    };
    let assign = |name: &str| {
        MirItem::TopLevelStmt(MirStmt::Assign {
            target: name.to_string(),
            value: MirExpr::IntLiteral(4_611_686_018_427_387_903),
        })
    };
    let ir = entry_ir(
        "module_fail_bigint",
        vec![
            assign("a"),
            assign("b"),
            module_try(
                vec![MirStmt::ExprStmt(MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::BinOp {
                        op: BinOpKind::Mul,
                        left: int_name("a"),
                        right: int_name("b"),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::ObjLen { base: copy_boxed() }),
                    ty: Ty::Int,
                })],
                vec![print_stmt(1)],
                Vec::new(),
                Vec::new(),
            ),
        ],
    );
    let start = ir
        .find("\nforeign_len_fail:")
        .unwrap_or_else(|| panic!("no foreign_len_fail block:\n{ir}"));
    let unwind = &ir[start..];
    let unwind = &unwind[..unwind
        .find("br label %try_handler_dispatch")
        .unwrap_or_else(|| panic!("the unwind must reach the handler dispatch:\n{ir}"))];
    let bridge = unwind
        .find(&format!("call i32 @{EXT_OBJ_ERROR_BRIDGE_SYMBOL}()"))
        .unwrap_or_else(|| panic!("the failure must be bridged:\n{ir}"));
    let release = unwind
        .find("call void @pycc_rt_bigint_release")
        .unwrap_or_else(|| panic!("the pending temporary must be released:\n{ir}"));
    assert!(bridge < release, "bridge first, then unwind:\n{ir}");
    assert!(!unwind.contains("ret i64"), "{ir}");
}
