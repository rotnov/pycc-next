//! Unit tests for `foreign_call.rs`, moved out of the parent file (Part 2a
//! of #1371) so that file stays under the ~1,000-line threshold.

use super::*;

mod call_tests;
mod keyword_tests;
use crate::{CompileOptions, EXT_MODULE_EXEC_SYMBOL, compile_to_object_with_observer};
use inkwell::values::AnyValue;
use pycc_mir::{MirExpr, MirItem, MirModule, MirStmt, Ty};

/// `import <module>` followed by one discarded
/// `<module>.<method>(args)` call.
///
/// A discarded `ExprStmt` was the only statement position PR 2b admitted
/// end to end -- `pycc_types` then refused binding a CPython object to
/// a name (`I0404`; admitted at module scope since #1325) -- so it is
/// the shape every test here builds.
fn call(module: &str, method: &str, args: Vec<MirExpr>) -> Vec<MirItem> {
    vec![
        MirItem::ForeignImport {
            local_name: module.to_string(),
            module_path: module.to_string(),
            from: None,
        },
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ObjMethodCall {
            base: Box::new(MirExpr::Name {
                name: module.to_string(),
                ty: Ty::Object,
            }),
            method: method.to_string(),
            args,
        })),
    ]
}

/// The LLVM text of the module-exec entry point after compiling `items`
/// as an `ext` object -- `foreign_attr.rs`'s own `entry_ir`, which is
/// where the rationale for compiling all the way to an object file
/// (LLVM's verifier runs before any assertion is believed) lives.
fn entry_ir(label: &str, items: Vec<MirItem>) -> String {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut ir = String::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _: Option<&'static str>| {
        if let Some(entry) = module.get_function(EXT_MODULE_EXEC_SYMBOL) {
            ir = crate::llvm_string_to_owned(entry.print_to_string());
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
    assert!(!ir.is_empty(), "no {EXT_MODULE_EXEC_SYMBOL} was emitted");
    ir
}

/// The zero-argument shape this PR's acceptance test builds
/// (`gc.disable()`): the call reaches the shim by its shared symbol,
/// the method name reaches it as a global string, and no packer is
/// emitted.
///
/// The symbol is asserted through [`EXT_OBJ_CALL_SYMBOL`] rather than
/// against a literal for `foreign_attr.rs`'s reason: the C definition
/// and this declaration are resolved lazily at load time, so a literal
/// spelled twice would be a crash at first call rather than a link
/// error.
#[test]
fn a_zero_argument_foreign_method_call_reaches_the_shim_by_its_shared_symbol() {
    let ir = entry_ir("foreign_call_zero_arg", call("gc", "disable", Vec::new()));
    assert!(ir.contains(EXT_OBJ_CALL_SYMBOL), "{ir}");
    assert!(ir.contains("pycc_foreign_method_disable"), "{ir}");
    assert!(!ir.contains(EXT_OBJ_PACK_INT_SYMBOL), "{ir}");
    assert!(
        ir.contains("i64 0"),
        "the arity is passed as a constant: {ir}"
    );
}

/// One packer per admitted scalar type, each exercised through its own
/// argument.
///
/// Written as four separate one-argument calls rather than one
/// four-argument call so a regression naming the wrong packer for one
/// type cannot hide behind another's symbol being present.
#[test]
fn each_admitted_argument_type_reaches_its_own_packer() {
    for (label, arg, symbol) in [
        ("int", MirExpr::IntLiteral(12345), EXT_OBJ_PACK_INT_SYMBOL),
        (
            "float",
            MirExpr::FloatLiteral(-1.0),
            EXT_OBJ_PACK_FLOAT_SYMBOL,
        ),
        ("bool", MirExpr::BoolLiteral(true), EXT_OBJ_PACK_BOOL_SYMBOL),
        (
            "str",
            MirExpr::StringLiteral("x".to_string()),
            EXT_OBJ_PACK_STR_SYMBOL,
        ),
    ] {
        let ir = entry_ir(
            &format!("foreign_call_arg_{label}"),
            call("gc", "set_threshold", vec![arg]),
        );
        assert!(ir.contains(symbol), "{label}: {ir}");
        assert!(ir.contains("foreign_call_args"), "{label}: {ir}");
        assert!(ir.contains("i64 1"), "the arity is one: {label}: {ir}");
    }
}

/// A failed call stops the module body on the module-exec failure edge,
/// rather than continuing with a `NULL` object.
///
/// This is the edge that makes a missing method surface as CPython's
/// own `AttributeError` and a raising method as its own exception,
/// instead of the `SystemError: execution of module n raised unreported
/// exception` PR 2a's review found when the edge was missing.
#[test]
fn a_failed_foreign_method_call_returns_on_the_module_exec_failure_edge() {
    let ir = entry_ir(
        "foreign_call_fail_edge",
        call("gc", "definitely_not_there", Vec::new()),
    );
    assert!(ir.contains("foreign_call_failed"), "{ir}");
    assert!(ir.contains("foreign_call_fail:"), "{ir}");
    assert!(ir.contains("foreign_call_cont:"), "{ir}");
    assert!(
        ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
        "{ir}"
    );
}

/// The *lookup*'s own failure edge, which the call edge above does not
/// cover: a missing method makes `pycc_ext_obj_getattr` return NULL
/// before any argument is packed, and that NULL must reach the same
/// `ret i64 -1`.
#[test]
fn a_failed_method_lookup_returns_on_the_module_exec_failure_edge() {
    let ir = entry_ir(
        "foreign_call_lookup_fail_edge",
        call("gc", "definitely_not_there", vec![MirExpr::IntLiteral(1)]),
    );
    assert!(ir.contains("foreign_call_lookup_failed"), "{ir}");
    assert!(ir.contains("foreign_call_lookup_fail:"), "{ir}");
    assert!(ir.contains("foreign_call_lookup_cont:"), "{ir}");
    assert!(
        ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
        "{ir}"
    );
}

/// CPython resolves a call's callable before it evaluates the
/// arguments, so `obj.missing(1 // 0)` raises `AttributeError` rather
/// than `ZeroDivisionError`. This pins that order where it is decided:
/// the `pycc_ext_obj_getattr` call site must precede every packer call
/// site in the emitted entry function.
#[test]
fn the_callable_is_resolved_before_any_argument_is_packed() {
    let ir = entry_ir(
        "foreign_call_lookup_order",
        call("gc", "set_threshold", vec![MirExpr::IntLiteral(1)]),
    );
    let lookup_at = ir
        .find(EXT_OBJ_GETATTR_SYMBOL)
        .unwrap_or_else(|| panic!("no method lookup was emitted: {ir}"));
    let pack_at = ir
        .find(EXT_OBJ_PACK_INT_SYMBOL)
        .unwrap_or_else(|| panic!("no argument packer was emitted: {ir}"));
    assert!(lookup_at < pack_at, "{ir}");
}

/// Two calls in one module share one extern declaration per symbol.
///
/// `shim_fn` returns the existing `FunctionValue` on every call after
/// the first; a second `add_function` of one name is an LLVM
/// module-verifier error, so the second call (which also repeats the
/// `int` packer) is what proves the early return is taken rather than
/// merely present.
#[test]
fn a_second_foreign_method_call_reuses_the_one_extern_declaration() {
    let mut items = call("gc", "set_threshold", vec![MirExpr::IntLiteral(1)]);
    items.extend(call("gc", "set_debug", vec![MirExpr::IntLiteral(2)]));
    let ir = entry_ir("foreign_call_twice", items);
    let count = |needle: &str| {
        let mut found = 0usize;
        let mut rest = ir.as_str();
        while let Some(at) = rest.find(needle) {
            found += 1;
            rest = &rest[at + needle.len()..];
        }
        found
    };
    assert_eq!(
        count(EXT_OBJ_CALL_SYMBOL),
        2,
        "one call site per call: {ir}"
    );
    assert_eq!(
        count(EXT_OBJ_PACK_INT_SYMBOL),
        2,
        "one packer call site per int argument: {ir}"
    );
}

/// The defensive arm in `foreign_attr::expect_object_pointer`, reached
/// through this node: only `pycc_mir`'s own lowering builds it, and only
/// over a `Ty::Object` base.
#[test]
#[should_panic(expected = "did not evaluate to a CPython object")]
fn a_non_object_base_is_an_internal_error() {
    entry_ir(
        "foreign_call_bad_base",
        vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(
            MirExpr::ObjMethodCall {
                base: Box::new(MirExpr::IntLiteral(1)),
                method: "disable".to_string(),
                args: Vec::new(),
            },
        ))],
    );
}

/// `import <module>` followed by one discarded `<module>[index]`
/// load -- the subscript counterpart of [`call`] above, and for the
/// same reason: a discarded `ExprStmt` is the only statement position
/// PR 3b admits end to end.
fn subscript(module: &str, index: MirExpr) -> Vec<MirItem> {
    vec![
        MirItem::ForeignImport {
            local_name: module.to_string(),
            module_path: module.to_string(),
            from: None,
        },
        MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ObjSubscript {
            base: Box::new(MirExpr::Name {
                name: module.to_string(),
                ty: Ty::Object,
            }),
            index: Box::new(index),
        })),
    ]
}

/// The subscript load reaches the shim by its shared symbol, and does
/// so without emitting the attribute lookup or the call helper: `o[k]`
/// is one `PyObject_GetItem`, not a `__getitem__` lookup followed by a
/// call.
///
/// The symbol is asserted through [`EXT_OBJ_GETITEM_SYMBOL`] rather
/// than against a literal for the same reason the call test gives: the
/// C definition and this declaration are resolved lazily at load time,
/// so a literal spelled twice would be a crash at first call rather
/// than a link error.
#[test]
fn a_foreign_subscript_reaches_the_shim_by_its_shared_symbol() {
    let ir = entry_ir(
        "foreign_subscript_symbol",
        subscript("gc", MirExpr::IntLiteral(0)),
    );
    assert!(ir.contains(EXT_OBJ_GETITEM_SYMBOL), "{ir}");
    assert!(ir.contains(EXT_OBJ_PACK_INT_SYMBOL), "{ir}");
    assert!(!ir.contains(EXT_OBJ_GETATTR_SYMBOL), "{ir}");
    assert!(!ir.contains(EXT_OBJ_CALL_SYMBOL), "{ir}");
}

/// One packer per admitted key type, each exercised through its own
/// key, so a regression naming the wrong packer for one type cannot
/// hide behind another's symbol being present.
///
/// `packer_for` is shared with the method-call path, but the *key*
/// slot is a second caller of it, and it is the slot whose ownership
/// rule the shim implements (`pycc_ext_obj_getitem` consumes the
/// reference the packer returns, on every path).
#[test]
fn each_admitted_key_type_reaches_its_own_packer() {
    for (label, key, symbol) in [
        ("int", MirExpr::IntLiteral(7), EXT_OBJ_PACK_INT_SYMBOL),
        (
            "float",
            MirExpr::FloatLiteral(1.5),
            EXT_OBJ_PACK_FLOAT_SYMBOL,
        ),
        ("bool", MirExpr::BoolLiteral(true), EXT_OBJ_PACK_BOOL_SYMBOL),
        (
            "str",
            MirExpr::StringLiteral("k".to_string()),
            EXT_OBJ_PACK_STR_SYMBOL,
        ),
    ] {
        let ir = entry_ir(
            &format!("foreign_subscript_key_{label}"),
            subscript("gc", key),
        );
        assert!(ir.contains(symbol), "{label}: {ir}");
        assert!(ir.contains("foreign_subscript_key"), "{label}: {ir}");
        assert!(ir.contains(EXT_OBJ_GETITEM_SYMBOL), "{label}: {ir}");
    }
}

/// A failed load stops the module body on the module-exec failure
/// edge, rather than continuing with a `NULL` object.
///
/// This is the one new `EXT_MODULE_EXEC_FAILED` edge PR 3b adds (outside
/// every module-level `try`; inside one it is bridged, Part 1 of #1096): a
/// failed *key packer* does not get its own, because
/// `pycc_ext_obj_getitem` tolerates a `NULL` key and returns `NULL`
/// itself, folding that case into this same branch.
#[test]
fn a_failed_foreign_subscript_returns_on_the_module_exec_failure_edge() {
    let ir = entry_ir(
        "foreign_subscript_fail_edge",
        subscript("gc", MirExpr::IntLiteral(0)),
    );
    assert!(ir.contains("foreign_subscript_failed"), "{ir}");
    assert!(ir.contains("foreign_subscript_fail:"), "{ir}");
    assert!(ir.contains("foreign_subscript_cont:"), "{ir}");
    assert!(
        ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
        "{ir}"
    );
}

/// `import <module>` followed by `for x in <module>.attr:` with a
/// body that consumes the loop variable.
///
/// `ObjLen` is the body statement because it is the one operation on
/// the loop variable that produces a value a discarded `ExprStmt` can
/// hold, so the target store is provably read rather than dead.
fn iter_loop(module: &str, body: Vec<MirStmt>) -> Vec<MirItem> {
    vec![
        MirItem::ForeignImport {
            local_name: module.to_string(),
            module_path: module.to_string(),
            from: None,
        },
        MirItem::TopLevelStmt(MirStmt::ForObject {
            var: "x".to_string(),
            iter: MirExpr::ObjAttrGet {
                base: Box::new(MirExpr::Name {
                    name: module.to_string(),
                    ty: Ty::Object,
                }),
                attr: "garbage".to_string(),
                ty: Ty::Object,
            },
            body,
        }),
    ]
}

/// The body used by every loop test here: `len(x)`, discarded.
fn iter_body() -> Vec<MirStmt> {
    vec![MirStmt::ExprStmt(MirExpr::ObjLen {
        base: Box::new(MirExpr::Name {
            name: "x".to_string(),
            ty: Ty::Object,
        }),
    })]
}

/// **The block structure of a `for x in <object>:` loop**, asserted as
/// a shape rather than as a set of symbols.
///
/// Six appended blocks, listed in the order emission creates them: the
/// `get_iter` NULL test's own fail/continuation pair (appended by the
/// shared `foreign_fail::route_null` helper), then the header, body, after and
/// next-failure blocks. The `get_iter` call itself is emitted into the
/// current block and appends none. The array below is the list; do not
/// restate its length in prose here or in `docs/RUNTIME.md`. Naming each one and asserting on the label is
/// what makes a regression that collapses two of them -- most
/// dangerously, routing exhaustion into the failure block -- fail here
/// instead of only in the `#[ignore]`d hosted test.
#[test]
fn a_foreign_for_loop_emits_its_full_block_structure() {
    let ir = entry_ir("foreign_iter_blocks", iter_loop("gc", iter_body()));
    for label in [
        "foreign_iter_get_fail:",
        "foreign_iter_get_cont:",
        "foreign_iter_header:",
        "foreign_iter_body:",
        "foreign_iter_after:",
        "foreign_iter_next_fail:",
    ] {
        assert!(ir.contains(label), "missing {label}: {ir}");
    }
    assert!(ir.contains(EXT_OBJ_GET_ITER_SYMBOL), "{ir}");
    assert!(ir.contains(EXT_OBJ_ITER_NEXT_SYMBOL), "{ir}");
}

/// **The three-way switch.** `1` enters the body, `0` leaves the loop,
/// and every other value -- `-1` and anything the shim could not
/// produce -- takes the failure block, because it is the `switch`'s
/// *default*.
///
/// Asserted against the emitted `switch` instruction itself, not
/// against a pair of comparisons: a regression that replaced the
/// switch with two `icmp`s could keep all six block labels above and
/// still send an unexpected status somewhere harmless.
#[test]
fn a_foreign_for_loop_switches_three_ways_on_the_iterator_status() {
    let ir = entry_ir("foreign_iter_switch", iter_loop("gc", iter_body()));
    let switch = ir
        .lines()
        .find(|line| line.trim_start().starts_with("switch i64 "))
        .unwrap_or_else(|| panic!("no switch instruction was emitted: {ir}"));
    assert!(
        switch.contains("label %foreign_iter_next_fail"),
        "the default destination is the failure block: {switch}"
    );
    let cases: String = ir
        .lines()
        .skip_while(|line| !line.trim_start().starts_with("switch i64 "))
        .take(4)
        .collect();
    assert!(
        cases.contains("i64 1, label %foreign_iter_body"),
        "a written item enters the body: {cases}"
    );
    assert!(
        cases.contains("i64 0, label %foreign_iter_after"),
        "clean exhaustion leaves the loop: {cases}"
    );
}

/// **Two** new `EXT_MODULE_EXEC_FAILED` edges outside every module-level
/// `try`, and exactly two: a `NULL` from `pycc_ext_obj_get_iter` and a
/// `-1` from `pycc_ext_obj_iter_next`. Clean exhaustion is
/// deliberately not one of them, which is why the shim helper is
/// three-valued rather than NULL-signalling.
#[test]
fn a_foreign_for_loop_adds_exactly_two_module_exec_failure_edges() {
    let ir = entry_ir("foreign_iter_fail_edges", iter_loop("gc", iter_body()));
    assert!(ir.contains("foreign_iter_get_failed"), "{ir}");
    // Each of the two named failure blocks carries exactly one
    // `ret i64 EXT_MODULE_EXEC_FAILED`, and they are the only blocks
    // this statement adds that do. Counted per named block rather than
    // over the whole entry point, because the iterable expression and
    // the loop body carry pre-existing edges of their own.
    for label in ["foreign_iter_get_fail:", "foreign_iter_next_fail:"] {
        let block: String = ir
            .lines()
            .skip_while(|line| !line.starts_with(label))
            .skip(1)
            .take_while(|line| !line.trim().is_empty())
            .collect();
        assert_eq!(
            block
                .matches(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}"))
                .count(),
            1,
            "{label} returns the module-exec failure value exactly once: {ir}"
        );
    }
}

/// The out-parameter is allocated **once**, in the entry block, not
/// once per iteration: an `alloca` inside the header would grow the
/// frame without bound on a long iteration, which is exactly the
/// defect [`alloca_in_entry_block`] exists to prevent.
#[test]
fn a_foreign_for_loop_allocates_its_out_parameter_in_the_entry_block() {
    let ir = entry_ir("foreign_iter_alloca", iter_loop("gc", iter_body()));
    let header_start = ir
        .find("foreign_iter_header:")
        .unwrap_or_else(|| panic!("no header block: {ir}"));
    assert!(
        !ir[header_start..].contains("alloca"),
        "no alloca may appear at or after the loop header: {ir}"
    );
    assert_eq!(
        ir.matches("= alloca ptr, i64 1").count(),
        1,
        "exactly one out-parameter slot is allocated: {ir}"
    );
}

/// A `Return` inside the loop body already terminates its block, so
/// the back-edge must not be built on top of it -- the same
/// terminator-safety guard `MirStmt::ForList` carries. LLVM's verifier
/// runs inside `entry_ir`, so a second terminator would fail the
/// compile rather than this assertion.
///
/// A module body cannot contain a `return`, so the terminating
/// statement here is a `raise`, which reaches the same guard.
#[test]
fn a_foreign_for_loop_body_that_already_terminates_gets_no_back_edge() {
    let ir = entry_ir(
        "foreign_iter_terminated_body",
        iter_loop("gc", vec![MirStmt::Unreachable]),
    );
    assert!(ir.contains("foreign_iter_after:"), "{ir}");
}

/// A codegen error raised *inside* the loop body propagates out of the
/// `ForObject` arm instead of being swallowed, exactly as the `ForList`
/// and `try*` arms propagate theirs.
///
/// `MirExceptionValue::Constructed` with a non-string message is the
/// standard way to make `emit_body` fail (`crates/pycc_codegen/src/
/// tests.rs`'s `a_codegen_error_in_a_try_star_body_propagates_...`),
/// and it is the only error `emit_body` can return at all.
#[test]
fn a_codegen_error_in_a_foreign_for_loop_body_propagates() {
    let dir = pycc_scratch::ScratchDir::new("foreign_iter_body_codegen_error")
        .expect("failed to create scratch dir");
    let err = compile_to_object_with_observer(
        &MirModule {
            items: iter_loop(
                "gc",
                vec![MirStmt::Raise {
                    exception: pycc_mir::MirExceptionValue::Constructed {
                        type_tag: 1,
                        class_name: "ValueError".to_string(),
                        message: MirExpr::IntLiteral(42),
                    },
                    frame_function: "test_fn".to_string(),
                }],
            ),
            ..Default::default()
        },
        &dir.join("foreign_iter_body_codegen_error.o"),
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
        None,
    )
    .expect_err("a codegen error in the loop body must propagate");
    assert!(err.contains("message must be a string"), "{err}");
}

/// The defensive arm in `foreign_attr::expect_object_pointer` reached
/// through the *loop* node, which has its own call to it.
#[test]
#[should_panic(expected = "did not evaluate to a CPython object")]
fn a_non_object_for_loop_iterable_is_an_internal_error() {
    entry_ir(
        "foreign_iter_bad_iterable",
        vec![MirItem::TopLevelStmt(MirStmt::ForObject {
            var: "x".to_string(),
            iter: MirExpr::IntLiteral(1),
            body: Vec::new(),
        })],
    );
}

/// The defensive arm in `foreign_attr::expect_object_pointer` reached
/// through the *subscript* node, which has its own call to it.
#[test]
#[should_panic(expected = "did not evaluate to a CPython object")]
fn a_non_object_subscript_base_is_an_internal_error() {
    entry_ir(
        "foreign_subscript_bad_base",
        vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(
            MirExpr::ObjSubscript {
                base: Box::new(MirExpr::IntLiteral(1)),
                index: Box::new(MirExpr::IntLiteral(0)),
            },
        ))],
    );
}

/// The defensive arm in [`packer_for`] reached through the key slot:
/// `pycc_types` refuses every key type that has no packer, so reaching
/// it is a front-end defect.
#[test]
#[should_panic(expected = "did not evaluate to a marshallable scalar")]
fn an_unmarshallable_subscript_key_is_an_internal_error() {
    entry_ir(
        "foreign_subscript_bad_key",
        subscript("gc", MirExpr::NoneLiteral),
    );
}

/// The defensive arm in [`packer_for`]: `pycc_types` refuses every
/// argument type that has no packer, so reaching it is a front-end
/// defect. Reached here by handing the node a list argument, which no
/// type-checked program produces (a `None` one is packed since Part 8 of
/// #1371, `keyword_tests.rs`).
#[test]
#[should_panic(expected = "did not evaluate to a marshallable scalar")]
fn an_unmarshallable_argument_is_an_internal_error() {
    entry_ir(
        "foreign_call_bad_arg",
        call(
            "gc",
            "disable",
            vec![MirExpr::ListLiteral(vec![MirExpr::IntLiteral(1)])],
        ),
    );
}
