//! Part 1 of #1092: the release of CPython object temporaries.
//!
//! Every IR test compiles real MIR as an `ext` object -- LLVM's verifier
//! runs before any assertion here is believed -- and reads the text of the
//! user function `f` (`pyfn_f`) or of the module-exec entry. The release is
//! a call to the shim's `pycc_ext_obj_release`; these tests pin where it is
//! emitted (after each borrowing consumer, and on every failure edge taken
//! while an operand is held) and where it is not (a borrowed name, a
//! consumed bound method, a value that is bound, returned or never
//! produced).

use super::*;
use crate::{
    CompileOptions, EXT_MODULE_EXEC_SYMBOL, EXT_OBJ_ERROR_BRIDGE_SYMBOL,
    compile_to_object_with_observer,
};
use inkwell::values::AnyValue;

use pycc_mir::{
    CmpOpKind, MirExceptHandler, MirItem, MirModule, MirStmt, ObjBuiltinClass, ObjIsInstanceClass,
    ObjKeywordCall, Ty,
};

const RELEASE: &str = "call void @pycc_ext_obj_release(";

fn import_copy() -> MirItem {
    MirItem::ForeignImport {
        local_name: "copy".to_string(),
        module_path: "copy".to_string(),
        from: None,
    }
}

/// `copy`: a borrowed module global.
fn copy_name() -> MirExpr {
    MirExpr::Name {
        name: "copy".to_string(),
        ty: Ty::Object,
    }
}

/// `copy.<attr>`: a produced new reference.
fn attr(attr: &str) -> MirExpr {
    attr_of(copy_name(), attr)
}

fn attr_of(base: MirExpr, attr: &str) -> MirExpr {
    MirExpr::ObjAttrGet {
        base: Box::new(base),
        attr: attr.to_string(),
        ty: Ty::Object,
    }
}

fn boxed(expr: MirExpr) -> Box<MirExpr> {
    Box::new(expr)
}

/// The LLVM text of the named functions after compiling `import copy` plus
/// `items` as an `ext` object.
fn functions_ir(label: &str, items: Vec<MirItem>, names: &[&str]) -> Vec<String> {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut all = vec![import_copy()];
    all.extend(items);
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
            items: all,
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

/// `def f() -> <return_ty>: <body>`.
fn function(return_ty: Ty, body: Vec<MirStmt>) -> MirItem {
    MirItem::Function {
        name: "f".to_string(),
        params: Vec::new(),
        return_ty,
        body,
    }
}

/// The IR of `f` running `body` and then `return`.
fn f_ir(label: &str, mut body: Vec<MirStmt>) -> String {
    body.push(MirStmt::Return(None));
    functions_ir(label, vec![function(Ty::None, body)], &["pyfn_f"]).remove(0)
}

/// The IR of `f` discarding `expr`.
fn discard_ir(label: &str, expr: MirExpr) -> String {
    f_ir(label, vec![MirStmt::ExprStmt(expr)])
}

/// The IR of the module-exec entry running `stmts` at module level.
fn entry_ir(label: &str, stmts: Vec<MirStmt>) -> String {
    let items = stmts.into_iter().map(MirItem::TopLevelStmt).collect();
    functions_ir(label, items, &[EXT_MODULE_EXEC_SYMBOL]).remove(0)
}

/// The text of every block whose label is `prefix` plus LLVM's uniquing
/// digits, in emission order, each up to its terminator.
fn blocks<'a>(ir: &'a str, prefix: &str) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut rest = ir;
    let header = format!("\n{prefix}");
    while let Some(at) = rest.find(&header) {
        let after = &rest[at + header.len()..];
        let label_end = after.find(':').unwrap_or(0);
        let suffix = &after[..label_end];
        let body_end = after.find("\n\n").unwrap_or(after.len());
        if suffix.chars().all(|c| c.is_ascii_digit()) {
            found.push(&after[label_end..body_end]);
        }
        rest = &after[label_end..];
    }
    found
}

fn releases(text: &str) -> usize {
    text.matches(RELEASE).count()
}

#[test]
fn every_shim_producer_is_produced_and_nothing_else_is() {
    let produced = [
        attr("a"),
        MirExpr::ObjMethodCall {
            base: boxed(copy_name()),
            method: "m".to_string(),
            args: Vec::new(),
        },
        MirExpr::ObjCall {
            callee: boxed(copy_name()),
            args: Vec::new(),
        },
        MirExpr::ObjKeywordCall(Box::new(ObjKeywordCall {
            call: MirExpr::ObjCall {
                callee: boxed(copy_name()),
                args: Vec::new(),
            },
            names: Vec::new(),
            values: Vec::new(),
        })),
        MirExpr::ObjSubscript {
            base: boxed(copy_name()),
            index: boxed(MirExpr::IntLiteral(0)),
        },
        MirExpr::ObjType {
            base: boxed(copy_name()),
        },
        MirExpr::ObjSlice {
            base: boxed(copy_name()),
            start: None,
            stop: None,
            step: None,
        },
        MirExpr::ObjList {
            elements: Vec::new(),
        },
        MirExpr::ObjUnpack {
            value: boxed(copy_name()),
            arity: 2,
        },
        MirExpr::ObjCompare {
            op: CmpOpKind::Lt,
            left: boxed(copy_name()),
            right: boxed(copy_name()),
        },
    ];
    for expr in &produced {
        assert!(is_produced(expr), "{expr:?}");
    }
    let borrowed = [
        copy_name(),
        MirExpr::ObjCompare {
            op: CmpOpKind::Is,
            left: boxed(copy_name()),
            right: boxed(MirExpr::NoneLiteral),
        },
        MirExpr::ObjLen {
            base: boxed(copy_name()),
        },
        MirExpr::IntLiteral(1),
    ];
    for expr in &borrowed {
        assert!(!is_produced(expr), "{expr:?}");
    }
}

/// `object_unbox::emit_pack_operand` evaluates the object inside an
/// `ObjectUnbox`, so the hold must classify that object, and a non-object
/// scalar is never held.
#[test]
fn an_unboxed_operand_is_classified_by_the_object_it_unboxes() {
    let context = Context::create();
    let module = context.create_module("object_release_unbox");
    let pointer = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    let object = Scalar::Object(pointer);
    let unboxed = |inner: MirExpr| MirExpr::ObjectUnbox(Box::new(inner), Box::new(Ty::Float));
    assert!(produced(&context, &module, &unboxed(attr("a")), &object).is_some());
    assert!(produced(&context, &module, &unboxed(copy_name()), &object).is_none());
    let float = Scalar::Float(context.f64_type().const_float(1.0));
    assert!(produced(&context, &module, &attr("a"), &float).is_none());
}

/// `copy.a.b`, discarded in a function: the inner load is released after
/// the outer one and on the outer one's failure edge before the branch; the
/// outer result is released by the discarding statement.
#[test]
fn a_nested_attribute_load_releases_its_base_on_both_paths() {
    let ir = discard_ir("release_nested_attr", attr_of(attr("a"), "b"));
    let fails = blocks(&ir, "foreign_attr_fail");
    assert_eq!(fails.len(), 2, "{ir}");
    assert_eq!(releases(fails[0]), 0, "nothing is held yet: {ir}");
    let outer = fails[1];
    let bridge = outer
        .find(&format!("call i32 @{EXT_OBJ_ERROR_BRIDGE_SYMBOL}()"))
        .unwrap_or_else(|| panic!("{ir}"));
    let release = outer.find(RELEASE).unwrap_or_else(|| panic!("{ir}"));
    let branch = outer
        .rfind("br label %exception_exit")
        .unwrap_or_else(|| panic!("{ir}"));
    assert!(bridge < release && release < branch, "{ir}");
    assert_eq!(releases(outer), 1, "{ir}");
    // The fallthrough: the base after the outer load, then the result.
    assert!(releases(&ir) >= 3, "{ir}");
}

/// At module level outside every `try`, the foreign failure returns the
/// module-exec failure status directly -- and releases what is held first.
#[test]
fn the_module_exec_failure_return_releases_held_operands() {
    let ir = entry_ir(
        "release_module_return",
        vec![MirStmt::ExprStmt(attr_of(attr("a"), "b"))],
    );
    let fails = blocks(&ir, "foreign_attr_fail");
    assert_eq!(fails.len(), 2, "{ir}");
    let outer = fails[1];
    let release = outer.find(RELEASE).unwrap_or_else(|| panic!("{ir}"));
    let ret = outer.find("ret ").unwrap_or_else(|| panic!("{ir}"));
    assert!(release < ret, "release before the failure return: {ir}");
    assert_eq!(releases(fails[0]), 0, "{ir}");
}

/// Inside a module-level `try`, the failure is bridged to the handler, and
/// the held operand is released on the way.
#[test]
fn a_failure_inside_a_module_level_try_releases_before_the_handler() {
    let ir = entry_ir(
        "release_module_try",
        vec![MirStmt::Try {
            body: vec![MirStmt::ExprStmt(attr_of(attr("a"), "b"))],
            handlers: vec![MirExceptHandler {
                exc_type_tag: None,
                binding_name: None,
                binding_ty: None,
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::IntLiteral(1)],
                    ty: Ty::None,
                })],
            }],
            orelse: Vec::new(),
            finalbody: Vec::new(),
        }],
    );
    let outer = blocks(&ir, "foreign_attr_fail")[1];
    let release = outer.find(RELEASE).unwrap_or_else(|| panic!("{ir}"));
    let branch = outer.rfind("br label %").unwrap_or_else(|| panic!("{ir}"));
    assert!(release < branch, "{ir}");
    assert!(!outer.contains("ret "), "{ir}");
}

/// `copy.m(copy.a)`: the bound method is held across the argument (so the
/// argument's failure releases it) and then consumed by the call (so the
/// call's failure does not release it); the produced argument is held
/// across the call and released after it.
#[test]
fn a_method_call_holds_its_bound_method_and_its_produced_argument() {
    let ir = discard_ir(
        "release_method_call",
        MirExpr::ObjMethodCall {
            base: boxed(copy_name()),
            method: "m".to_string(),
            args: vec![attr("a")],
        },
    );
    let lookup = blocks(&ir, "foreign_call_lookup_fail");
    assert_eq!(lookup.len(), 1, "{ir}");
    assert_eq!(releases(lookup[0]), 0, "a borrowed receiver: {ir}");
    let arg = blocks(&ir, "foreign_attr_fail");
    assert_eq!(arg.len(), 1, "{ir}");
    assert_eq!(releases(arg[0]), 1, "the bound method: {ir}");
    let call = blocks(&ir, "foreign_call_fail");
    assert_eq!(call.len(), 1, "{ir}");
    assert_eq!(
        releases(call[0]),
        1,
        "only the argument; the call consumed the bound method: {ir}"
    );
}

/// `copy.a.m()`: a produced receiver is released once the lookup is done
/// and on the lookup's own failure edge.
#[test]
fn a_method_call_releases_a_produced_receiver_after_the_lookup() {
    let ir = discard_ir(
        "release_method_receiver",
        MirExpr::ObjMethodCall {
            base: boxed(attr("a")),
            method: "m".to_string(),
            args: Vec::new(),
        },
    );
    let lookup = blocks(&ir, "foreign_call_lookup_fail");
    assert_eq!(releases(lookup[0]), 1, "{ir}");
    let call = blocks(&ir, "foreign_call_fail");
    assert_eq!(releases(call[0]), 0, "the receiver is gone already: {ir}");
}

/// Borrowed operands only: a borrowed name is never released, and the
/// consumed bound method is released only where the call never runs -- the
/// argument read's own `NameError` exit (`global_unbound`). The one other
/// release is the discarded result's.
#[test]
fn borrowed_operands_and_a_consumed_bound_method_are_never_released() {
    let ir = discard_ir(
        "release_borrowed_only",
        MirExpr::ObjMethodCall {
            base: boxed(copy_name()),
            method: "m".to_string(),
            args: vec![copy_name(), MirExpr::IntLiteral(1)],
        },
    );
    assert_eq!(releases(blocks(&ir, "foreign_call_fail")[0]), 0, "{ir}");
    let unbound: usize = blocks(&ir, "global_unbound")
        .iter()
        .map(|block| releases(block))
        .sum();
    assert_eq!(unbound, 1, "the bound method, on the argument's exit: {ir}");
    assert_eq!(releases(&ir), 2, "{ir}");
}

/// A produced value that is bound or returned is not a temporary: it is
/// never released as one. Since Part 1 of #1499 a module global owns its
/// value, so the only releases left are each global's *previous* value on
/// its rebind branch (`object_slot.rs`) -- the fixture's `copy` import and
/// `x` -- never the value just bound.
#[test]
fn a_bound_or_returned_produced_value_is_not_released() {
    let ir = entry_ir(
        "release_bound",
        vec![MirStmt::Assign {
            target: "x".to_string(),
            value: attr("a"),
        }],
    );
    let rebinds = blocks(&ir, "global_release_old");
    assert_eq!(rebinds.len(), 2, "{ir}");
    for rebind in &rebinds {
        assert_eq!(releases(rebind), 1, "{ir}");
        assert!(
            rebind.contains(&format!("{RELEASE}ptr %global_old")),
            "{ir}"
        );
    }
    assert_eq!(releases(&ir), rebinds.len(), "{ir}");
    let ir = functions_ir(
        "release_returned",
        vec![function(Ty::Object, vec![MirStmt::Return(Some(attr("a")))])],
        &["pyfn_f"],
    )
    .remove(0);
    assert_eq!(releases(&ir), 0, "{ir}");
}

/// A function without an object operation declares no release and keeps
/// its guard shape: the empty pending stack costs nothing.
#[test]
fn a_function_without_object_operations_emits_no_release() {
    let ir = f_ir(
        "release_none",
        vec![MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::IntLiteral(1)],
            ty: Ty::None,
        })],
    );
    assert!(!ir.contains("pycc_ext_obj_release"), "{ir}");
}

/// Every consumer releases its produced operands: each statement below is
/// compiled on its own and emits exactly `expected` releases, summed over
/// the fallthrough and every failure edge taken while an operand is held.
///
/// The counts are pinned exactly because over-release is the dangerous
/// direction. `subscript`'s 8, read off the IR: `copy` unbound, `b`'s load
/// failing and the effect guard after it each release `a` (3); the
/// subscript's own failure edge and its fallthrough each release `b` then
/// `a` (4); the discarded result is released last (1). Each path from entry
/// to a return releases each produced value at most once; the hosted
/// `tests/issue_1092_object_temp_release.rs` measures that balance.
#[test]
fn every_consumer_releases_its_produced_operands() {
    let call = |callee: &str, ty: Ty| MirExpr::Call {
        callee: callee.to_string(),
        args: vec![attr("a")],
        ty,
    };
    let cases: Vec<(&str, MirStmt, usize)> = vec![
        (
            "len",
            MirStmt::ExprStmt(MirExpr::ObjLen {
                base: boxed(attr("a")),
            }),
            2,
        ),
        (
            "type",
            MirStmt::ExprStmt(MirExpr::ObjType {
                base: boxed(attr("a")),
            }),
            3,
        ),
        (
            "subscript",
            MirStmt::ExprStmt(MirExpr::ObjSubscript {
                base: boxed(attr("a")),
                index: boxed(attr("b")),
            }),
            8,
        ),
        (
            "compare",
            MirStmt::ExprStmt(MirExpr::ObjCompare {
                op: CmpOpKind::Lt,
                left: boxed(attr("a")),
                right: boxed(attr("b")),
            }),
            8,
        ),
        (
            "identity",
            MirStmt::ExprStmt(MirExpr::ObjCompare {
                op: CmpOpKind::Is,
                left: boxed(attr("a")),
                right: boxed(MirExpr::NoneLiteral),
            }),
            1,
        ),
        (
            "contains",
            MirStmt::ExprStmt(MirExpr::ObjContains {
                negate: false,
                item: boxed(attr("a")),
                container: boxed(attr("b")),
            }),
            7,
        ),
        (
            "slice",
            MirStmt::ExprStmt(MirExpr::ObjSlice {
                base: boxed(attr("a")),
                start: Some(boxed(attr("b"))),
                stop: None,
                step: Some(boxed(MirExpr::IntLiteral(2))),
            }),
            8,
        ),
        (
            "list",
            MirStmt::ExprStmt(MirExpr::ObjList {
                elements: vec![attr("a"), MirExpr::IntLiteral(1), attr("b")],
            }),
            8,
        ),
        (
            "isinstance_object",
            MirStmt::ExprStmt(MirExpr::ObjIsInstance {
                value: boxed(attr("a")),
                class: ObjIsInstanceClass::Object(boxed(attr("b"))),
            }),
            7,
        ),
        (
            "isinstance_builtin",
            MirStmt::ExprStmt(MirExpr::ObjIsInstance {
                value: boxed(attr("a")),
                class: ObjIsInstanceClass::Builtin(ObjBuiltinClass::Int),
            }),
            2,
        ),
        (
            "unpack",
            MirStmt::ExprStmt(MirExpr::ObjUnpack {
                value: boxed(attr("a")),
                arity: 2,
            }),
            3,
        ),
        (
            "direct_call",
            MirStmt::ExprStmt(MirExpr::ObjCall {
                callee: boxed(attr("a")),
                args: vec![attr("b")],
            }),
            6,
        ),
        (
            "keyword_method_call",
            MirStmt::ExprStmt(MirExpr::ObjKeywordCall(Box::new(ObjKeywordCall {
                call: MirExpr::ObjMethodCall {
                    base: boxed(attr("a")),
                    method: "m".to_string(),
                    args: vec![attr("b")],
                },
                names: vec!["k".to_string()],
                values: vec![attr("c")],
            }))),
            16,
        ),
        (
            "keyword_direct_call",
            MirStmt::ExprStmt(MirExpr::ObjKeywordCall(Box::new(ObjKeywordCall {
                call: MirExpr::ObjCall {
                    callee: boxed(attr("a")),
                    args: Vec::new(),
                },
                names: vec!["k".to_string()],
                values: vec![attr("c")],
            }))),
            6,
        ),
        ("not", MirStmt::ExprStmt(MirExpr::Not(boxed(attr("a")))), 2),
        ("float", MirStmt::ExprStmt(call("float", Ty::Float)), 2),
        ("bool", MirStmt::ExprStmt(call("bool", Ty::Bool)), 2),
        ("int", MirStmt::ExprStmt(call("int", Ty::Int)), 2),
        ("str", MirStmt::ExprStmt(call("str", Ty::Str)), 2),
        (
            "if",
            MirStmt::If {
                test: attr("a"),
                body: vec![MirStmt::Return(None)],
                orelse: Vec::new(),
            },
            2,
        ),
        (
            "while",
            MirStmt::While {
                test: attr("a"),
                body: vec![MirStmt::Return(None)],
            },
            2,
        ),
        (
            "del_attr",
            MirStmt::ObjDelAttr {
                base: attr("a"),
                attr: "x".to_string(),
            },
            2,
        ),
        (
            "del_slice",
            MirStmt::ObjDelSlice {
                base: attr("a"),
                start: Some(attr("b")),
                stop: None,
                step: None,
            },
            7,
        ),
        (
            "set_attr",
            MirStmt::ObjAttrSet {
                base: attr("a"),
                attr: "x".to_string(),
                value: attr("b"),
            },
            5,
        ),
    ];
    let mut wrong = Vec::new();
    for (label, stmt, expected) in cases {
        let ir = f_ir(&format!("release_consumer_{label}"), vec![stmt]);
        let found = releases(&ir);
        if found != expected {
            wrong.push(format!("{label}: expected {expected}, found {found}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The module-level-only `a, b = o.pair` float-tuple unpack releases its
/// produced base.
#[test]
fn a_module_level_float_tuple_unpack_releases_its_base() {
    let ir = entry_ir(
        "release_float_tuple",
        vec![MirStmt::ExprStmt(MirExpr::ObjUnpackFloatTuple {
            base: boxed(attr("pair")),
            arity: 2,
        })],
    );
    assert!(releases(&ir) >= 1, "{ir}");
}

/// The hold is the fallthrough's to retire: a hold that went missing from
/// the stack is an internal error, never a silent double release.
#[test]
#[should_panic(expected = "missing from pending_object_releases")]
fn retiring_a_hold_twice_is_an_internal_error() {
    let context = Context::create();
    let module = context.create_module("object_release_twice");
    let rt = crate::tests::list_scalar_panic_fixture(&context).1;
    let pointer = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    let held = hold_new_reference(&context, &module, &rt, pointer);
    let twin = Held(held.0);
    held.consumed(&rt);
    twin.consumed(&rt);
}

/// `1 // <divisor>`: a native expression that raises `ZeroDivisionError`
/// through pycc's own exception state, never through a foreign failure
/// edge.
fn native_raise(divisor: MirExpr) -> MirExpr {
    MirExpr::BinOp {
        op: pycc_mir::BinOpKind::FloorDiv,
        left: boxed(MirExpr::IntLiteral(1)),
        right: boxed(divisor),
        ty: Ty::Int,
    }
}

/// `z`: the `int` parameter of the function the native-raise tests build,
/// so the divisor is a runtime value no pass can fold.
fn z() -> MirExpr {
    MirExpr::Name {
        name: "z".to_string(),
        ty: Ty::Int,
    }
}

/// #1486: a *native* raise in a method call's argument (`copy.m(1 // z)`)
/// leaves through `guard_statement_effects`' `effect_exc_unwind` block,
/// which releases the held bound method -- and every produced argument
/// evaluated before it -- before the branch to the exception target. The
/// call's own failure edge still releases only the produced arguments: the
/// call consumed the bound method. The keyword path holds the bound method
/// across its keyword values the same way.
#[test]
fn a_native_raise_in_an_argument_releases_the_bound_method() {
    let method_call = |args: Vec<MirExpr>| MirExpr::ObjMethodCall {
        base: boxed(copy_name()),
        method: "m".to_string(),
        args,
    };
    let keyword_call = |args: Vec<MirExpr>| {
        MirExpr::ObjKeywordCall(Box::new(ObjKeywordCall {
            call: method_call(args),
            names: vec!["k".to_string()],
            values: vec![native_raise(z())],
        }))
    };
    // (label, call, releases on the native unwind, on the call's failure).
    let cases = [
        (
            "release_native_arg",
            method_call(vec![native_raise(z())]),
            1,
            0,
        ),
        (
            "release_native_later_arg",
            method_call(vec![attr("a"), native_raise(z())]),
            2,
            1,
        ),
        ("release_native_keyword", keyword_call(Vec::new()), 1, 0),
        (
            "release_native_keyword_after_arg",
            keyword_call(vec![attr("a")]),
            2,
            1,
        ),
    ];
    for (label, call, unwind_releases, call_releases) in cases {
        let ir = functions_ir(
            label,
            vec![MirItem::Function {
                name: "f".to_string(),
                params: vec![("z".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![MirStmt::ExprStmt(call), MirStmt::Return(None)],
            }],
            &["pyfn_f"],
        )
        .remove(0);
        // A produced earlier argument has a guard of its own; every guard
        // taken while the bound method is held releases it, and the
        // native raise's guard is the last one before the call.
        let unwind = blocks(&ir, "effect_exc_unwind");
        assert_eq!(unwind.len(), unwind_releases, "{label}\n{ir}");
        for block in &unwind {
            let bound = format!("{RELEASE}ptr %foreign_call_bound)");
            assert_eq!(block.matches(&bound).count(), 1, "{label}\n{ir}");
        }
        let native = unwind.last().expect("the native raise's guard");
        let release = native.find(RELEASE).unwrap_or_else(|| panic!("{ir}"));
        let branch = native.rfind("br label %").unwrap_or_else(|| panic!("{ir}"));
        assert!(release < branch, "{label}: release, then branch\n{ir}");
        assert_eq!(releases(native), unwind_releases, "{label}\n{ir}");
        let call_fail = blocks(&ir, "foreign_call_fail");
        assert_eq!(call_fail.len(), 1, "{label}\n{ir}");
        assert_eq!(releases(call_fail[0]), call_releases, "{label}\n{ir}");
    }
}

/// The direct-call form of #1486: `copy.a(copy.b, 1 // z)` holds its
/// produced callee and its produced earlier argument across the native
/// raise, and the last unwind releases both before its branch.
#[test]
fn a_native_raise_in_an_argument_releases_a_produced_callee() {
    let ir = functions_ir(
        "release_native_callee",
        vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![("z".to_string(), Ty::Int)],
            return_ty: Ty::None,
            body: vec![
                MirStmt::ExprStmt(MirExpr::ObjCall {
                    callee: boxed(attr("a")),
                    args: vec![attr("b"), native_raise(z())],
                }),
                MirStmt::Return(None),
            ],
        }],
        &["pyfn_f"],
    )
    .remove(0);
    let unwind = blocks(&ir, "effect_exc_unwind");
    let native = unwind.last().unwrap_or_else(|| panic!("{ir}"));
    assert_eq!(releases(native), 2, "{ir}");
    let release = native.find(RELEASE).unwrap_or_else(|| panic!("{ir}"));
    let branch = native.rfind("br label %").unwrap_or_else(|| panic!("{ir}"));
    assert!(release < branch, "{ir}");
}

/// #1486 at module level: outside every `try`, the native raise's unwind
/// branches to the module's top exception exit, and still releases the
/// held bound method first.
#[test]
fn a_module_level_native_raise_in_an_argument_releases_the_bound_method() {
    let ir = entry_ir(
        "release_native_arg_module",
        vec![MirStmt::ExprStmt(MirExpr::ObjMethodCall {
            base: boxed(copy_name()),
            method: "m".to_string(),
            args: vec![native_raise(MirExpr::IntLiteral(0))],
        })],
    );
    let unwind = blocks(&ir, "effect_exc_unwind");
    assert_eq!(unwind.len(), 1, "{ir}");
    assert_eq!(releases(unwind[0]), 1, "the bound method\n{ir}");
}

#[path = "object_release_iteration_tests.rs"]
mod iteration;

#[path = "object_release_operand_tests.rs"]
mod operand;
