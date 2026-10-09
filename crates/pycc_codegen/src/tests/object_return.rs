//! #1387: `None` returned from an `object`-returning function
//! (`object_return.rs`), pinned on the LLVM IR of an `ext` object.
//!
//! The ownership half of the contract is structural, since CPython's `None`
//! is immortal and a reference count proves nothing: the body hands back
//! the *borrowed* pointer `pycc_ext_obj_none` returns, with no reference
//! traffic of its own, and the export wrapper's `pycc_ext_pack_object`
//! takes the new reference the host receives.

use super::*;

fn function(name: &str, body: Vec<MirStmt>) -> MirItem {
    MirItem::Function {
        name: name.to_string(),
        params: vec![("flag".to_string(), Ty::Bool)],
        return_ty: Ty::Object,
        body,
    }
}

/// The text of `pyfn_<name>`'s definition in `ir`.
fn body_of<'ir>(ir: &'ir str, name: &str) -> &'ir str {
    let header = format!("@pyfn_{name}(");
    let start = ir
        .lines()
        .position(|line| line.starts_with("define") && line.contains(&header))
        .unwrap_or_else(|| panic!("`pyfn_{name}` should be defined: {ir}"));
    let rest: Vec<&str> = ir.lines().skip(start).collect();
    let end = rest
        .iter()
        .position(|line| *line == "}")
        .expect("a closing brace");
    let first = rest[0];
    let offset = ir.find(first).expect("the header is in the IR");
    let len: usize = rest[..=end].iter().map(|line| line.len() + 1).sum();
    &ir[offset..offset + len - 1]
}

#[test]
fn a_bare_return_and_return_none_hand_back_cpython_s_none() {
    let items = vec![
        function("bare", vec![MirStmt::Return(None)]),
        function("literal", vec![MirStmt::Return(Some(MirExpr::NoneLiteral))]),
        // Routed through a `finally`: the value is stored to the frame's
        // return slot rather than returned directly, and must still be the
        // `None` pointer.
        function(
            "via_finally",
            vec![MirStmt::Try {
                body: vec![MirStmt::Return(None)],
                handlers: Vec::new(),
                orelse: Vec::new(),
                finalbody: vec![MirStmt::ExprStmt(MirExpr::IntLiteral(1))],
            }],
        ),
    ];
    compile_ext_items_checking_ir("object_return_none", items, |ir| {
        for name in ["bare", "literal", "via_finally"] {
            let body = body_of(ir, name);
            assert!(
                body.contains("call ptr @pycc_ext_obj_none()"),
                "{name}: {body}"
            );
            // Never the type's zero default, which the export wrapper would
            // report as a `SystemError`, and never the native `Optional`
            // carrier `NoneLiteral` is everywhere else. (The frame's
            // `exception_exit` block returns null on a pending exception,
            // which is the ordinary failure convention, not this return.)
            let normal = body.split("exception_exit:").next().expect("a prefix");
            assert!(!normal.contains("ret ptr null"), "{name}: {body}");
            assert!(!body.contains("{ i8, i8 }"), "{name}: {body}");
            // Borrowed: the body itself adds no reference.
            assert!(!body.contains("pycc_ext_obj_pack_object"), "{name}: {body}");
        }
    });
}

#[test]
fn a_bare_return_in_a_native_function_keeps_its_default_value() {
    // The `object` rule keys on the declared return type: an `int`
    // function's bare `return` (the abstract-method body, #380) still
    // returns the type's zero default and names no CPython `None`.
    let items = vec![MirItem::Function {
        name: "native".to_string(),
        params: Vec::new(),
        return_ty: Ty::Int,
        body: vec![MirStmt::Return(None)],
    }];
    compile_ext_items_checking_ir("object_return_native", items, |ir| {
        let body = body_of(ir, "native");
        assert!(!body.contains("pycc_ext_obj_none"), "{body}");
        assert!(body.contains("ret i64 "), "{body}");
    });
}
