//! Part 8 of #1371: a call with keyword arguments on a CPython object,
//! `MirExpr::ObjKeywordCall`, and the `None` argument every object call
//! now packs.
//!
//! `use super::*` reaches the parent's private `call` and `entry_ir`
//! helpers.

use super::*;
use pycc_mir::ObjKeywordCall;

/// `import gc` followed by one discarded keyword call whose positional
/// half is `positional` (built by the parent's `call`).
fn keyword_call(positional: Vec<MirItem>, names: &[&str], values: Vec<MirExpr>) -> Vec<MirItem> {
    let mut items = positional;
    let Some(MirItem::TopLevelStmt(MirStmt::ExprStmt(call))) = items.pop() else {
        panic!("the positional half is a discarded call");
    };
    items.push(MirItem::TopLevelStmt(MirStmt::ExprStmt(
        MirExpr::ObjKeywordCall(Box::new(ObjKeywordCall {
            call,
            names: names.iter().map(|name| name.to_string()).collect(),
            values,
        })),
    )));
    items
}

/// The positional half of `gc(args)`: a direct call of the module global.
fn direct(args: Vec<MirExpr>) -> Vec<MirItem> {
    let mut items = call("gc", "unused", Vec::new());
    items.pop();
    items.push(MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ObjCall {
        callee: Box::new(MirExpr::Name {
            name: "gc".to_string(),
            ty: Ty::Object,
        }),
        args,
    })));
    items
}

/// A method keyword call looks the method up, consumes the bound method
/// through `pycc_ext_obj_call_kw`, passes one positional slot (the total
/// minus the keyword count), and hands the keyword names over as global
/// strings in source order.
#[test]
fn a_method_keyword_call_consumes_the_bound_method_and_names_its_keywords() {
    let ir = entry_ir(
        "foreign_kw_method",
        keyword_call(
            call("gc", "collect", vec![MirExpr::IntLiteral(1)]),
            &["generation", "flag"],
            vec![MirExpr::IntLiteral(2), MirExpr::BoolLiteral(true)],
        ),
    );
    assert!(ir.contains(EXT_OBJ_GETATTR_SYMBOL), "{ir}");
    assert!(ir.contains(&format!("@{EXT_OBJ_CALL_KW_SYMBOL}(")), "{ir}");
    assert!(
        !ir.contains(&format!("@{EXT_OBJ_CALL_KW_BORROWED_SYMBOL}(")),
        "{ir}"
    );
    assert!(ir.contains("foreign_call_kwname_slot"), "{ir}");
    let generation = ir.find("pycc_foreign_kwname_generation").expect(&ir);
    let flag = ir.find("pycc_foreign_kwname_flag").expect(&ir);
    assert!(generation < flag, "names keep source order: {ir}");
    assert!(ir.contains("i64 1, ptr"), "one positional slot: {ir}");
    assert!(ir.contains("i64 2)"), "two keyword names: {ir}");
}

/// A direct keyword call of a module global borrows it, exactly as the
/// positional direct call does.
#[test]
fn a_direct_keyword_call_of_a_global_borrows_the_callee() {
    let ir = entry_ir(
        "foreign_kw_direct",
        keyword_call(direct(Vec::new()), &["k"], vec![MirExpr::IntLiteral(3)]),
    );
    assert!(
        ir.contains(&format!("@{EXT_OBJ_CALL_KW_BORROWED_SYMBOL}(")),
        "{ir}"
    );
    assert!(!ir.contains(&format!("@{EXT_OBJ_CALL_KW_SYMBOL}(")), "{ir}");
    assert!(!ir.contains(EXT_OBJ_GETATTR_SYMBOL), "{ir}");
    assert!(ir.contains("i64 0, ptr"), "no positional slot: {ir}");
}

/// A `None` argument, positional or keyword, packs CPython's own `None`
/// as an object; a positional-only call keeps the keyword-free shim.
#[test]
fn a_none_argument_packs_cpythons_none() {
    let ir = entry_ir(
        "foreign_kw_none",
        keyword_call(
            direct(vec![MirExpr::NoneLiteral]),
            &["k"],
            vec![MirExpr::NoneLiteral],
        ),
    );
    assert!(ir.contains(EXT_OBJ_NONE_SYMBOL), "{ir}");
    assert!(ir.contains(EXT_OBJ_PACK_OBJECT_SYMBOL), "{ir}");

    let ir = entry_ir(
        "foreign_none_positional",
        call("gc", "collect", vec![MirExpr::NoneLiteral]),
    );
    assert!(ir.contains(EXT_OBJ_NONE_SYMBOL), "{ir}");
    assert!(ir.contains(&format!("@{EXT_OBJ_CALL_SYMBOL}(")), "{ir}");
    assert!(!ir.contains(EXT_OBJ_CALL_KW_SYMBOL), "{ir}");
}

/// `pycc_mir` only builds a keyword call around an `ObjMethodCall` or an
/// `ObjCall`; any other positional half is a front-end defect.
#[test]
#[should_panic(expected = "a keyword call's positional half must be")]
fn a_keyword_call_around_another_node_is_an_internal_error() {
    entry_ir(
        "foreign_kw_bad_half",
        vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(
            MirExpr::ObjKeywordCall(Box::new(ObjKeywordCall {
                call: MirExpr::IntLiteral(1),
                names: vec!["k".to_string()],
                values: vec![MirExpr::IntLiteral(2)],
            })),
        ))],
    );
}
